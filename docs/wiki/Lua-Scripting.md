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
Lua. A long-running function blocks input; to do something later, use a [timer](#timers)
instead of waiting.

### Bind options

`neori.bind` takes the bind's options as an optional third argument, whatever the action is:

```lua
neori.bind("Mod+Ctrl+P", function()
    local out = neori.focused_output()
    if out then neori.action("spawn", { "wl-mirror", out:name() }) end
end, { hotkey_overlay_title = "Mirror Display", ["repeat"] = false })

neori.bind("Mod+Return", neori.spawn("kitty"), { hotkey_overlay_title = "Terminal" })
neori.bind("Mod+Q", "close-window", { ["repeat"] = false })
```

The options are `repeat`, `cooldown_ms`, `allow_when_locked`, `allow_inhibiting`,
`pass_through`, `keycode` and `hotkey_overlay_title`. An unknown name is an error, and so is
setting an option both there and in a table action (`{ close_window = true, … }` also accepts
the options as keys).

Without options, a function bind repeats while the key is held, shows a generic label in the
hotkey overlay, and doesn't run while the session is locked or during a screenshot, since the
compositor can't predict what the function will do. Set `allow_when_locked = true` if you need
it.

A bind can run different functions on press and release:
`neori.bind("Mod+Space", { press = function() … end, release = function() … end })`.

Lua's built-in `print(...)` is overridden to append to a log file; see
[`print(...)` output](#print-output) below.

## `neori.action(name, args)` — dispatch an IPC action from Lua

Inside a Lua function bind (or a Lua event handler — see below), run any niri-IPC
action. It happens right away, so the next line of your function already sees the result.

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

Actions run in the order you call them. If the callback errors afterwards, the actions it
already ran stay done.

Changes only work inside a callback: calling `neori.action`, `neori.warp` or a mutation
method while the config file itself is being evaluated raises an error, since there is no
compositor to apply them to yet. Use `neori.on("startup", function() … end)` for things to
do at startup.

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
| `workspace-removed`  | `Workspace` userdata — last known state of the destroyed workspace |
| `output-focused`     | `Output` userdata — the newly active output         |
| `layout-changed`     | string — the new active keyboard layout's name      |
| `device-added`       | `Device` — an input device was connected (after the config's input settings were applied to it) |
| `device-removed`     | `Device` — its last known state                     |
| `layer-added`        | `LayerSurface` — a layer-shell surface (panel, bar, launcher, …) was mapped |
| `layer-removed`      | `LayerSurface` — its last known state; also fires when the app unmaps it with a null buffer |
| `mode-changed`       | the new mode's name or `nil`, the old mode's name or `nil` — see [keybinding modes](Configuration:-Key-Bindings.md#keybinding-modes); `neori.current_mode()` returns the active one |
| `config-reloaded`    | — (no arg); runs in the newly loaded config, after a successful reload |
| `shutdown`           | — (no arg); the compositor is about to exit         |
| `window-title-changed`      | `Window`, its old title                      |
| `window-fullscreen-changed` | `Window`, whether it was fullscreen          |
| `window-floating-changed`   | `Window`, whether it was floating            |
| `window-urgent-changed`     | `Window`, whether it was urgent              |
| `window-workspace-changed`  | `Window`, its old workspace (`{ idx, name }`) |
| `workspace-name-changed`    | `Workspace`, its old name                    |
| `output-mode-changed`       | `Output`, its old mode                       |

The `*-changed` events are *property signals*: they fire when a property of an existing object
changes (not when it appears), with the object, which reads the new value, and the old value.
They're checked once per frame, only while something listens to one of them.

```lua
-- Mention windows that ask for attention.
neori.on("window-urgent-changed", function(w, was_urgent)
    if w:is_urgent() then
        neori.action("spawn", { "notify-send", "Needs attention", w:title() or "?" })
    end
end)
```

Workspace / output lifecycle events fire whenever the compositor notices a
change (per refresh tick), so a bind that focuses a different workspace,
plugging in a monitor, or opening a window on a not-yet-materialized workspace
all generate the matching events.

## Environment

`neori.setenv(name, value)` and `neori.unsetenv(name)` change the environment of the programs
neori spawns from then on, like `neori.environment { … }` does in the config. They only work
inside callbacks, and last until the config is reloaded, which puts back the config's
environment. `neori.getenv(name)` returns a variable as those programs would see it (Lua's own
`os.getenv` shows neori's environment, without these changes).

```lua
-- Toggle between a light and a dark GTK theme for apps started from now on.
neori.bind("Mod+Alt+D", function()
    local dark = neori.getenv("GTK_THEME") == "Adwaita:dark"
    neori.setenv("GTK_THEME", dark and "Adwaita" or "Adwaita:dark")
end)
```

## Timers

`neori.after(ms, fn)` calls `fn` once, after `ms` milliseconds. `neori.every(ms, fn)` calls it
every `ms` milliseconds until the timer is stopped. Both return a `Timer`, and `fn` receives it
too, so a callback can stop or restart its own timer.

```lua
-- Remind me to stretch every 50 minutes.
neori.every(50 * 60 * 1000, function()
    neori.action("spawn", { "notify-send", "Stand up and stretch" })
end)

-- Start the bar a moment after login, once the outputs have settled.
neori.on("startup", function()
    neori.after(2000, function() neori.action("spawn", { "waybar" }) end)
end)

-- Notify five seconds after the last press of a key: each press restarts the countdown.
local quiet = neori.after(5000, function() neori.action("spawn", { "notify-send", "quiet now" }) end)
quiet:stop()
neori.bind("Mod+Alt+H", function() quiet:again() end)
```

| method              | does                                                               |
|---                  |---                                                                 |
| `t:start()`         | Start a stopped timer. A running one keeps its countdown.          |
| `t:again()`         | Start the countdown again from the full timeout.                   |
| `t:stop()`          | Stop the timer.                                                    |
| `t:is_running()`    | Whether it's running.                                              |
| `t:timeout()`       | The timeout, in milliseconds.                                      |
| `t:set_timeout(ms)` | Change the timeout, from the next `start` or `again`.              |

Timers can be made anywhere, including at the top level of the config: they start once the
config has loaded. A config reload drops the old config's timers. Timer callbacks run like any
other Lua callback: their changes apply right away, an error shows the config error
notification, and a repeating timer keeps running after an error.

## Signals

The events above are *signals*, and `neori.on(event, function)` is one way to connect to them.
`neori.connect` is the general form: it also takes signals of your own, and returns the function
so you can disconnect it later.

| function                        | does                                                        |
|---                              |---                                                          |
| `neori.connect(name, fn)`       | Connects `fn` to the signal `name` and returns `fn`.        |
| `neori.disconnect(name, fn)`    | Disconnects `fn`. Returns whether it was connected.         |
| `neori.disconnect(name)`        | Disconnects every function connected to `name`.             |
| `neori.emit(name, ...)`         | Calls every function connected to `name` with `...`, now.   |

Any name that isn't one of the events above is a custom signal, which only runs when you emit
it. `neori.on` only accepts the events above, so a typo there is an error rather than a signal
that never fires.

```lua
-- A custom signal: one place to react to "presentation mode" toggles.
presenting = false
neori.connect("presentation", function(on)
    neori.action("spawn-sh", on and "makoctl mode -a dnd" or "makoctl mode -r dnd")
end)
neori.bind("Mod+Alt+P", function()
    presenting = not presenting
    neori.emit("presentation", presenting)
end)

-- Connect, then disconnect after the first time. (Declare the local first: it isn't
-- in scope inside its own initializer.)
local first_open
first_open = neori.connect("window-opened", function(w)
    print("first window:", w:app_id())
    neori.disconnect("window-opened", first_open)
end)
```

`emit` runs every connected function even if one fails, then raises the first error to its
caller.

### Delivery order

A built-in signal never interrupts Lua code that is already running. If one fires while a
callback runs (for example because of a change the callback made), it waits and is delivered
right after the callback, in the order the signals fired. `window-opening` is the
exception: the window waits for its handlers, so they always run immediately.

A signal delivered that way can cause more signals, which wait their turn too. If such a chain
goes more than 16 levels deep, the next signal is dropped, a warning is logged and the config
error notification appears, so handlers that keep causing each other's signals can't hang the
compositor.

This only covers signals caused while Lua runs. Most built-in signals fire on the next frame
instead (focus, workspace and output changes are noticed once per frame), and each frame starts
over. So two `window-focused` handlers that keep focusing each other's window don't hit the
limit: they bounce focus back and forth on every frame until one of them stops.

`neori.emit` is different: it calls the handlers of your own signals directly, like a function
call.

### Listing signals

`neori.signals()` returns an array describing every built-in signal, in the order of the table
above, followed by every custom signal that has handlers, sorted by name. Each entry is a table:

| field             | meaning                                                                  |
|---                |---                                                                       |
| `name`            | the signal's name                                                        |
| `builtin`         | `true` for signals the compositor emits, `false` for custom ones         |
| `handlers`        | how many functions are connected to it                                   |
| `description`     | what happened (built-in signals only)                                    |
| `args`            | what the functions receive (built-in signals only)                       |
| `object_signal`   | the short name it has on objects, like `closed` (built-in signals that reach objects) |
| `object_handlers` | how many functions are connected to `object_signal`, across all objects  |

`obj:signals()` lists the signals of one `Window`, `Workspace` or `Output` the same way: its
built-in signals by their short names, then its custom signals, with `handlers` counting that
object's functions only.

```lua
-- Print every signal that something listens to, to the Lua log.
neori.bind("Mod+Alt+L", function()
    for _, s in ipairs(neori.signals()) do
        local objects = s.object_handlers or 0
        if s.handlers > 0 or objects > 0 then
            print(string.format("%-18s %d global, %d on objects", s.name, s.handlers, objects))
        end
    end
end)
```

### Object signals

`Window`, `Workspace` and `Output` values have the same three methods, for signals attached to
that one object: `obj:connect(name, fn)`, `obj:disconnect(name[, fn])` and `obj:emit(name, ...)`.
Their functions receive the object first, then the arguments passed to `emit`.

The built-in events also reach the object they're about, under a short name:

| object      | signals                                                                     |
|---          |---                                                                          |
| `Window`    | `focused`, `closed` (from `window-focused`, `window-closed`), and `title-changed`, `fullscreen-changed`, `floating-changed`, `urgent-changed`, `workspace-changed` |
| `Workspace` | `focused`, `removed`, `name-changed` (from `workspace-focused`, …)          |
| `Output`    | `focused` (from `output-focused`), `mode-changed`; `added`, `removed` (from `monitor-added`, `monitor-removed`, called with the connector name and the `Output`, like the global event) |
| `Device`    | `removed` (from `device-removed`); its functions are then dropped, since another device may get the same sysname |

```lua
-- Get a notification when this particular window closes.
neori.bind("Mod+Alt+W", function()
    local w = neori.focused_window()
    if w then
        w:connect("closed", function(win)
            neori.action("spawn-sh", string.format("notify-send 'closed' %q", win:title() or "?"))
        end)
    end
end)
```

A window's functions are disconnected when it closes, and a workspace's when it's removed.
Output functions are keyed by connector name, so they keep working when the monitor is
unplugged and plugged back in. Inside `window-opening` the window doesn't exist yet, so
`w:connect` raises an error there; connect from `window-opened` instead.

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

### Sections as namespaces

The sections above that take a table (`input`, `layout`, `cursor`, `animations`, `gestures`,
`switch_events`, `recent_windows`, `hotkey_overlay`, `overview`, `blur`, `clipboard`,
`config_notification`, `xwayland_satellite`, `environment` and `debug`) can also be read and
assigned field by field:

```lua
neori.layout { gaps = 10, focus_ring = { width = 4 } }
neori.layout.gaps = 16                 -- set one field
neori.layout.focus_ring.width = 2      -- nested fields work once the subsection is set
print(neori.layout.gaps)               --> 16
neori.cursor.xcursor_size = 32         -- creates the section if needed
neori.input = { keyboard = { numlock = true } }  -- same as calling neori.input { … }
```

A read returns what the config has set so far, or `nil` for a field it hasn't set (it doesn't
know neori's defaults). Inside callbacks it returns what the loaded config set.

Calling a section with a table merges it into what was set before, following the rules on
[Configuration: Include](Configuration:-Include.md#merging): subsections merge field by field,
while lists, gradients and a few combined settings (like `struts`) are replaced. Assigning a
field (`neori.layout.focus_ring = { … }`) replaces just that field.

The options that aren't sections live in `neori.misc`, which works the same way:

```lua
neori.misc.prefer_no_csd = true
neori.misc.screenshot_path = nil       -- don't save screenshots (reads back as false)
neori.misc { prefer_no_csd = true, screenshot_path = "~/Pictures/%Y-%m-%d.png" }
print(neori.misc.prefer_no_csd)        --> true
```

`neori.prefer_no_csd(true)` and `neori.screenshot_path("…")` keep working and set the same
options. Assigning to an API function (`neori.prefer_no_csd = true`) is an error, since it
would replace the function and set nothing.

The plural `neori.outputs` / `neori.workspaces` / `neori.windows` are **runtime lookup
functions** — they return the current objects and are documented in the sections
below. There is no `neori.outputs(table)` / `neori.workspaces(table)` section setter;
declare entries one by one with the singular form instead.

### Parse-time utilities

| call                                                           | what it does                            |
|---                                                             |---                                      |
| `neori.set("key", value)`                                      | low-level: write directly into the config data table (escape hatch for a field no section setter covers) |
| `neori.is_startup()`                                           | `true` while the compositor's first config loads and its `startup` handlers run; `false` in a reloaded config |
| `neori.strict(true)` / `neori.strict(false)` / `neori.strict()` | toggle strict parsing — unknown config keys become errors instead of warnings |
| `neori.include("other.lua", "snippets/*.lua", "~/dir/")`       | evaluate other Lua files in the same environment; see [Configuration: Include](Configuration:-Include.md) for path rules |

## Windows

Inside any Lua callback (bind or event), `neori.windows()` returns an array of the
mapped windows, and `neori.focused_window()` returns the focused one or `nil`.
`neori.window_at(x, y)` returns the window at those global coordinates, for example the one under
the pointer: `neori.window_at(neori.pointer().x, neori.pointer().y)`.
While the config file itself is being evaluated there are no windows yet: they return
an empty array and `nil`.
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

Read-only accessors return the window's current state, even for a `Window` you kept in
a variable from an earlier callback. Once the window is closed, they return the last
state the `Window` value was created with.

Mutation methods run an IPC action against the window's stable id, right away: after
`w:fullscreen(true)`, `w:is_fullscreen()` returns `true`. (The app itself catches up a
moment later; `is_fullscreen` and `is_floating` report what the compositor decided.)
An action for a window that has since closed is silently ignored, so it's safe to keep a
Window in a Lua variable and call methods on it across callbacks.

**Read-only:**

| method              | returns                                                    |
|---                  |---                                                         |
| `w:id()`            | stable numeric id (persists across focus/workspace moves)  |
| `w:app_id()`        | string or `nil`                                            |
| `w:title()`         | string or `nil`                                            |
| `w:is_focused()`    | boolean                                                    |
| `w:is_floating()`   | boolean                                                    |
| `w:is_fullscreen()` | boolean                                                    |
| `w:is_urgent()`     | boolean — whether the window requests attention            |
| `w:is_maximized()`  | boolean — maximized to the edges of its output             |
| `w:pid()`           | the app's process id, or `nil` (always `nil` for X11 windows, which all belong to xwayland-satellite) |
| `w:geometry()`      | `{ x, y, width, height }` in global logical coordinates; `x` and `y` are `nil` while the window isn't on screen |
| `w:workspace()`     | `{ idx = <number>, name = <string?> }` or `nil`            |
| `w:output()`        | connector-name string or `nil`                             |
| `w:opacity()`       | number between 0 and 1 — the opacity it's drawn with       |
| `w:parent()`        | the `Window` this one is a dialog or child of, or `nil`    |

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
| `w:float()`                   | Toggle between floating and tiling.                       |
| `w:maximize(true/false/())`   | Maximize to the output's edges, restore, or toggle (`maximize-window-to-edges`). |
| `w:center()`                  | Center the window on its output.                          |
| `w:kill()`                    | End the app's process with SIGKILL, for an app that ignores `w:close()`. Errors for X11 windows, whose process serves every X11 app. |
| `w:set_opacity(x)`            | Draw the window with opacity `x` (0 to 1), over any window rule's; `w:set_opacity(nil)` goes back to the rule. Kept until the window closes. |
| `w:send_to_workspace(n)`      | Move to workspace index `n` (1-based).                   |
| `w:send_to_workspace("name")` | Move to the named workspace.                             |
| `w:send_to_output("eDP-1")`   | Move to the named output.                                |

Set-style calls (`w:fullscreen(true)`, `w:float(false)`, …) do nothing when the window
already is in the requested state, so calling them repeatedly is safe.

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
position as `{ x = …, y = …, output = <connector-name or nil> }`. While the config
file itself is being evaluated it returns `nil`, and so does `neori.keyboard_layout()`.
The coordinates are in the compositor's global logical space — the same space
window / output `position` fields use. `neori.warp(x, y)` moves the cursor to
those coordinates right away.

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
touch panels). Each one reads the device's current metadata and capability bits; mutation
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
| `d:set_accel_profile(s)`         | `"adaptive"` or `"flat"`.                                        |
| `d:set_scroll_method(s)`         | `"no-scroll"`, `"two-finger"`, `"edge"` or `"on-button-down"`.   |
| `d:set_click_method(s)`          | `"clickfinger"` or `"button-areas"`.                             |
| `d:set_dwt(bool)`                | Disable-while-typing, for touchpads.                             |
| `d:set_middle_emulation(bool)`   | Middle click by pressing the left and right buttons together.    |

Mutations go directly to libinput; they persist on the device until the next
config reload re-applies the static `input { … }` block, at which point the
config value takes over again. Not every setting is supported by every
device — libinput silently ignores unsupported calls.

## Layer surfaces

`neori.layers()` returns an array of `LayerSurface` userdata — one per mapped layer-shell
surface: panels, bars, launchers, notification popups, wallpapers, lock screens and so on.
`layer-added` and `layer-removed` fire when one is mapped or goes away.

```lua
-- Restart the bar if it crashes.
neori.on("layer-removed", function(ls)
    if ls:namespace() == "waybar" then
        neori.after(1, function() neori.spawn({ "waybar" }) end)
    end
end)

-- Close every overlay-layer surface.
neori.bind("Mod+Alt+Escape", function()
    for _, ls in ipairs(neori.layers()) do
        if ls:layer() == "overlay" then ls:close() end
    end
end)
```

### `LayerSurface` methods

| Method | Returns |
|---|---|
| `id()` | integer; a surface that's unmapped and mapped again gets a new id |
| `namespace()` | string the app gave it, like `"waybar"`; layer rules match on it |
| `layer()` | `"background"`, `"bottom"`, `"top"` or `"overlay"` |
| `output()` | connector name of its output, or `nil` once that output is gone |
| `pid()` | integer process id, or `nil` |
| `keyboard_interactivity()` | `"none"`, `"exclusive"` or `"on-demand"` |
| `close()` | asks the app to close the surface |
| `kill()` | ends the app's process with SIGKILL (refused for pid 1 and neori itself) |

Reads return the surface's current state, or its last known one once it's gone.
`LayerSurface` is a signal object: `ls:connect("removed", fn)` runs `fn` when it goes away,
and its handlers are dropped then.

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
and `neori.focused_output()` returns the focused one (or `nil`). `neori.output_at(x, y)` returns
the output at those global coordinates. The
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

Read-only accessors return the output's current state, even for an `Output` kept in a
variable from an earlier callback. Once the output is turned off or unplugged, they
return its last known state, and `out:is_off()` returns `true`. Mutation methods
apply an `OutputAction` right away: after `out:set_scale(2)`, `out:scale()` returns `2`.

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
| `out:geometry()`       | `{ x, y, width, height }` — position and logical size, in global logical coordinates |
| `out:workarea()`       | `{ x, y, width, height }` — the area left for windows once layer-shell panels and bars reserved their space |
| `out:physical_size()`  | `{ width, height }` in millimetres, or `nil` if unknown (like for virtual outputs) |
| `out:windows()`        | array of the `Window`s on this output                         |

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

Read-only accessors return the workspace's current state, even for a `Workspace` kept
in a variable from an earlier callback. Once the workspace is removed, they return its
last known state. Mutation methods run an `Action` right away, just like window
mutations.

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
workspace has been destroyed since, the action is ignored — Lua methods never raise
for that case.

## Running Lua from a shell

`neori msg lua` runs Lua code in the compositor, in the config's Lua state, and prints what it
returns:

```sh
neori msg lua 'neori.focused_window():title()'
# Terminal
neori msg lua 'neori.signals()'
# { { args = "nothing", builtin = true, description = "the compositor started", … }, … }
neori msg lua 'for _, w in ipairs(neori.windows()) do print(w:app_id(), w:title()) end'
neori msg lua -f ~/snippet.lua
```

The code runs like a callback: reads are live, changes apply right away, and the config's
globals and functions are there, so you can call helpers your config defined. An expression
prints its values; statements can `return` values. What the code `print`s comes back to the
terminal instead of the Lua log, and an error is reported with a non-zero exit code. `--json`
prints `{ "printed": …, "values": [ … ] }`.

In zsh, Tab completes the code: `neori.` names, methods after `:` (`neori.focused_window():ti`
completes to `title`), and global names, including your config's own. The types come from the
[editor definitions](#editor-support), and nothing you typed is run to work them out.

The code can change anything a config callback can, so only processes of your own user can use
it, through the IPC socket, like `neori msg action spawn`.

## Editor support

neori ships type definitions of this API for [lua-language-server](https://luals.github.io/)
(LuaLS), which most editors use for Lua. With them, the editor completes `neori.*` functions and
the methods of `Window`, `Output`, `Workspace` and `Device`, knows what each event handler
receives (`neori.on("window-closed", function(w) … end)` knows `w` is a `Window`), and warns
about typos like `neori.fucused_window()`.

The definitions are installed to `share/neori/lua/neori.lua` (on NixOS,
`/run/current-system/sw/share/neori/lua`), and live in `resources/lua/neori.lua` in the source
tree. Point LuaLS at that directory with a `.luarc.json` next to your config:

```json
{
  "runtime.version": "Lua 5.4",
  "workspace.library": ["/run/current-system/sw/share/neori/lua"]
}
```

## Known limitations (today)

- Handlers that keep undoing each other across frames aren't stopped: a `window-focused`
  handler that always focuses another window makes focus bounce on every frame.
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

-- A tiny "scratchpad"-ish pattern: toggle a named kitty.
--
-- For real scratchpads (hide/show, spawn-on-empty, several by name) use the built-in
-- `neori.scratchpad` + `toggle-scratchpad` action instead — see the Key-Bindings page.
-- This example is just Lua-held state: on the first press, spawn the window with a known
-- class; on subsequent presses flip whether it's floating.
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
