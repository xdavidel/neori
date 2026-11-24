<!-- Generated from resources/lua/neori.lua by neori-config/tests/lua-reference.rs; don't edit. Update it with:
     NEORI_UPDATE_LUA_REFERENCE=1 cargo test -p neori-config --test lua-reference -->

Every function, method, type and signal of neori's Lua API, generated from the type definitions that editors use (see [Editor support](./Lua-Scripting.md#editor-support)). For how to use them, with examples, see [Lua Scripting](./Lua-Scripting.md). The keys of the config sections are described on the Configuration pages.

- [Shared types](#shared-types)
- [neori](#neori)
- [Window](#window)
- [Output](#output)
- [Workspace](#workspace)
- [Timer](#timer)
- [LayerSurface](#layersurface)
- [Device](#device)

## Shared types

#### `neori.Signal`

A signal that the compositor emits.

| value | |
|---|---|
| `"startup"` | The compositor started. |
| `"monitor-added"` | A monitor was connected. |
| `"monitor-removed"` | A monitor was disconnected. |
| `"lid-open"` | The laptop lid was opened. |
| `"lid-closed"` | The laptop lid was closed. |
| `"tablet-mode-on"` | Tablet mode was turned on. |
| `"tablet-mode-off"` | Tablet mode was turned off. |
| `"window-opening"` | A window is about to open; `w:open_*` methods set where. |
| `"window-opened"` | A window opened. |
| `"window-closed"` | A window closed. |
| `"window-focused"` | A window got keyboard focus. |
| `"workspace-focused"` | A workspace became the active one. |
| `"workspace-added"` | A workspace was created. |
| `"workspace-removed"` | A workspace was removed. |
| `"output-focused"` | An output became the active one. |
| `"layout-changed"` | The keyboard layout changed. |
| `"device-added"` | An input device was connected. |
| `"device-removed"` | An input device was disconnected. |
| `"layer-added"` | A layer-shell surface (a panel, bar, launcher, …) appeared. |
| `"layer-removed"` | A layer-shell surface went away. |
| `"pointer-button"` | A mouse button was pressed or released. |
| `"pointer-axis"` | The pointer scrolled. |
| `"gesture-swipe-begin"` | A touchpad swipe started. |
| `"gesture-swipe-update"` | A touchpad swipe moved. |
| `"gesture-swipe-end"` | A touchpad swipe ended. |
| `"gesture-pinch-begin"` | A touchpad pinch started. |
| `"gesture-pinch-update"` | A touchpad pinch moved. |
| `"gesture-pinch-end"` | A touchpad pinch ended. |
| `"gesture-hold-begin"` | Fingers were held still on the touchpad. |
| `"gesture-hold-end"` | A touchpad hold ended. |
| `"mode-changed"` | A keybinding mode was entered or left. |
| `"config-reloaded"` | The config was reloaded; runs in the newly loaded config. |
| `"shutdown"` | The compositor is about to exit. |
| `"window-title-changed"` | A window's title changed. |
| `"window-fullscreen-changed"` | A window entered or left fullscreen. |
| `"window-floating-changed"` | A window moved between the floating and the tiling layout. |
| `"window-urgent-changed"` | A window started or stopped requesting attention. |
| `"window-workspace-changed"` | A window moved to another workspace. |
| `"workspace-name-changed"` | A workspace was named, renamed or unnamed. |
| `"output-mode-changed"` | An output's mode changed. |

#### `neori.Action`

An action: an action name like `"close-window"` (with its arguments, as in
`"focus-workspace 2"`), or an action table like `{ focus_workspace = 2 }` or the result of
`neori.spawn`.

Type: `string|table`

#### `neori.BindOptions`

Options of a bind, the third argument of `neori.bind`.

| field | type | |
|---|---|---|
| `repeat?` | `boolean` | Repeat the action while the key is held. Defaults to true, unless the bind has a release action. |
| `cooldown_ms?` | `integer` | Ignore the bind for this many milliseconds after it fires. |
| `allow_when_locked?` | `boolean` | Also run while the session is locked. |
| `allow_inhibiting?` | `boolean` | Let apps inhibit the bind (keyboard shortcuts inhibitor). Defaults to true. |
| `pass_through?` | `boolean` | Also send the key to the focused app. |
| `keycode?` | `boolean` | Match the physical key (keycode) instead of the symbol of the current layout. |
| `hotkey_overlay_title?` | `string\|false` | Title in the hotkey overlay, or `false` to hide the bind from it. |

#### `neori.PressRelease`

A bind whose actions run on press and on release.

| field | type | |
|---|---|---|
| `press?` | `neori.Action\|fun()` |  |
| `release?` | `neori.Action\|fun()` |  |

#### `neori.Point`

A position in the compositor's global logical space.

| field | type | |
|---|---|---|
| `x` | `number` |  |
| `y` | `number` |  |

#### `neori.Pointer`

| field | type | |
|---|---|---|
| `output?` | `string` | Connector name of the output under the pointer. |

#### `neori.WindowGeometry`

Where a window is, in the compositor's global logical space.

| field | type | |
|---|---|---|
| `x` | `number?` | Position of the top-left corner; nil while the window isn't on screen. |
| `y` | `number?` |  |
| `width` | `integer` |  |
| `height` | `integer` |  |

#### `neori.WindowWorkspace`

The workspace of a window.

| field | type | |
|---|---|---|
| `idx` | `integer` | Global 1-based index. |
| `name?` | `string` |  |

#### `neori.PointerButtonEvent`

What `pointer-button` handlers receive.

| field | type | |
|---|---|---|
| `button` | `integer` | The evdev button code, like 272 (`BTN_LEFT`). |
| `name?` | `"left"\|"right"\|"middle"\|"side"\|"extra"\|"forward"\|"back"` |  |
| `pressed` | `boolean` |  |
| `x` | `number` | Pointer position, in global logical coordinates. |
| `y` | `number` |  |

#### `neori.PointerAxisEvent`

What `pointer-axis` handlers receive.

| field | type | |
|---|---|---|
| `dx` | `number` | Horizontal scroll amount. |
| `dy` | `number` | Vertical scroll amount. |
| `dx_v120?` | `number` | Horizontal wheel steps, 120 per notch; nil for touchpads. |
| `dy_v120?` | `number` | Vertical wheel steps, 120 per notch; nil for touchpads. |
| `source` | `"wheel"\|"finger"\|"continuous"\|"wheel-tilt"` |  |
| `x` | `number` | Pointer position, in global logical coordinates. |
| `y` | `number` |  |

#### `neori.GestureBeginEvent`

What `gesture-*-begin` handlers receive.

| field | type | |
|---|---|---|
| `fingers` | `integer` |  |

#### `neori.GestureSwipeUpdateEvent`

What `gesture-swipe-update` handlers receive.

| field | type | |
|---|---|---|
| `dx` | `number` |  |
| `dy` | `number` |  |

#### `neori.GesturePinchUpdateEvent`

What `gesture-pinch-update` handlers receive.

| field | type | |
|---|---|---|
| `dx` | `number` |  |
| `dy` | `number` |  |
| `scale` | `number` | Relative to the start of the pinch: 1 is unchanged. |
| `rotation` | `number` | Degrees since the last update, clockwise. |

#### `neori.GestureEndEvent`

What `gesture-*-end` handlers receive.

| field | type | |
|---|---|---|
| `cancelled` | `boolean` |  |

#### `neori.SignalInfo`

A signal, as listed by `neori.signals()` and `obj:signals()`.

| field | type | |
|---|---|---|
| `name` | `string` |  |
| `builtin` | `boolean` | True for signals that the compositor emits, false for custom ones. |
| `handlers` | `integer` | How many functions are connected to it. |
| `description?` | `string` | What happened (built-in signals only). |
| `args?` | `string` | What the functions receive (built-in signals only). |
| `object_signal?` | `string` | The short name it has on objects, like `"closed"` (`neori.signals()`, for built-in signals that reach objects). |
| `object_handlers?` | `integer` | How many functions are connected to `object_signal`, across all objects. |

## neori

### Binds and actions

#### `neori.bind(key, action, options?)`

Binds a key to an action, a Lua function, or press and release actions. In the config file a key
can be bound once; bound again later, from a callback or `neori msg lua`, the new bind replaces
the old one.

```lua
neori.bind("Mod+Q", "close-window")
neori.bind("Mod+Return", neori.spawn("kitty"), { hotkey_overlay_title = "Terminal" })
neori.bind("Mod+F1", function() print("pressed") end, { ["repeat"] = false })
```

Parameters:

- `key` `string`: A key combination, like `"Mod+Shift+E"` or `"Mod+WheelScrollDown"`.
- `action` `neori.Action|neori.PressRelease|fun()`
- `options?` `neori.BindOptions`

#### `neori.action(name, args?)`

Runs an action right away. Only works inside a callback.

```lua
neori.action("focus-workspace", 2)
neori.action("spawn", { "kitty", "-e", "htop" })
```

Parameters:

- `name` `string`: An action name, in kebab or snake case.
- `args?` `any`: The action's argument: a value, a sequence, or a table of fields.

#### `neori.spawn(...)`

Builds a `spawn` action that runs a program directly. A trailing table of bind options is
merged in.

Parameters:

- `...` `string|string[]|neori.BindOptions`: The command and its arguments.

Returns `table`.

#### `neori.spawn_sh(command, options?)`

Builds a `spawn` action that runs a command through `sh -c`.

Parameters:

- `command` `string`
- `options?` `neori.BindOptions`

Returns `table`.

#### `neori.create_virtual_output(options?)`

Builds a `create-virtual-output` action.

Parameters:

- `options?` `{ name?: string, width?: integer, height?: integer, refresh_rate?: integer }`

Returns `table`.

#### `neori.remove_virtual_output(name)`

Builds a `remove-virtual-output` action.

Parameters:

- `name` `string|{ name: string }`

Returns `table`.

#### `neori.warp(x, y)`

Moves the cursor to global logical coordinates. Only works inside a callback.

Parameters:

- `x` `number`
- `y` `number`

### Keybinding modes

#### `neori.mode(name, declare)`

Declares a keybinding mode: the binds made with `neori.bind` while `declare` runs belong to the
mode. While the mode is active, they replace the normal binds; give it a bind that leaves it.
Declared again after the config loaded, the mode is replaced.

```lua
neori.mode("resize", function()
    neori.bind("Left", "set-column-width -10%")
    neori.bind("Escape", "exit-mode")
end)
neori.bind("Mod+R", "enter-mode resize")
```

Parameters:

- `name` `string`
- `declare` `fun()`

#### `neori.enter_mode(name)`

Enters a keybinding mode, like the `enter-mode` action.

Parameters:

- `name` `string`

#### `neori.exit_mode()`

Leaves the active keybinding mode, like the `exit-mode` action.

#### `neori.current_mode()`

The name of the active keybinding mode, or nil.

Returns `string?`.

### Timers

#### `neori.after(ms, callback)`

Calls `callback` once, after `ms` milliseconds. Returns the timer, which the callback also
receives. Timers made while the config loads start once it has loaded; a config reload
drops the old config's timers.

Parameters:

- `ms` `integer`
- `callback` `fun(timer: neori.Timer)`

Returns `neori.Timer`.

#### `neori.every(ms, callback)`

Calls `callback` every `ms` milliseconds, until the timer is stopped.

Parameters:

- `ms` `integer`: At least 1.
- `callback` `fun(timer: neori.Timer)`

Returns `neori.Timer`.

### Signals

#### `neori.on(event, handler)`

Connects a function to a built-in event. Like `neori.connect`, but only takes built-in
events, and also takes a string or table action instead of a function.

Parameters:

- `event` `neori.Signal`
- `handler` `function|neori.Action`

Can also be called as:

```
fun(event: "startup", handler: fun()|neori.Action)
fun(event: "monitor-added", handler: fun(connector: string, output: neori.Output)|neori.Action)
fun(event: "monitor-removed", handler: fun(connector: string, output: neori.Output)|neori.Action)
fun(event: "lid-open", handler: fun()|neori.Action)
fun(event: "lid-closed", handler: fun()|neori.Action)
fun(event: "tablet-mode-on", handler: fun()|neori.Action)
fun(event: "tablet-mode-off", handler: fun()|neori.Action)
fun(event: "window-opening", handler: fun(window: neori.Window))
fun(event: "window-opened", handler: fun(window: neori.Window))
fun(event: "window-closed", handler: fun(window: neori.Window))
fun(event: "window-focused", handler: fun(window: neori.Window))
fun(event: "workspace-focused", handler: fun(workspace: neori.Workspace))
fun(event: "workspace-added", handler: fun(workspace: neori.Workspace))
fun(event: "workspace-removed", handler: fun(workspace: neori.Workspace))
fun(event: "output-focused", handler: fun(output: neori.Output))
fun(event: "layout-changed", handler: fun(layout: string))
fun(event: "device-added", handler: fun(device: neori.Device))
fun(event: "device-removed", handler: fun(device: neori.Device))
fun(event: "layer-added", handler: fun(layer: neori.LayerSurface))
fun(event: "layer-removed", handler: fun(layer: neori.LayerSurface))
fun(event: "pointer-button", handler: fun(event: neori.PointerButtonEvent))
fun(event: "pointer-axis", handler: fun(event: neori.PointerAxisEvent))
fun(event: "gesture-swipe-begin", handler: fun(event: neori.GestureBeginEvent))
fun(event: "gesture-swipe-update", handler: fun(event: neori.GestureSwipeUpdateEvent))
fun(event: "gesture-swipe-end", handler: fun(event: neori.GestureEndEvent))
fun(event: "gesture-pinch-begin", handler: fun(event: neori.GestureBeginEvent))
fun(event: "gesture-pinch-update", handler: fun(event: neori.GesturePinchUpdateEvent))
fun(event: "gesture-pinch-end", handler: fun(event: neori.GestureEndEvent))
fun(event: "gesture-hold-begin", handler: fun(event: neori.GestureBeginEvent))
fun(event: "gesture-hold-end", handler: fun(event: neori.GestureEndEvent))
fun(event: "mode-changed", handler: fun(mode: string?, old_mode: string?))
fun(event: "config-reloaded", handler: fun()|neori.Action)
fun(event: "shutdown", handler: fun()|neori.Action)
fun(event: "window-title-changed", handler: fun(window: neori.Window, old_title: string?))
fun(event: "window-fullscreen-changed", handler: fun(window: neori.Window, was_fullscreen: boolean))
fun(event: "window-floating-changed", handler: fun(window: neori.Window, was_floating: boolean))
fun(event: "window-urgent-changed", handler: fun(window: neori.Window, was_urgent: boolean))
fun(event: "window-workspace-changed", handler: fun(window: neori.Window, old_workspace: neori.WindowWorkspace?))
fun(event: "workspace-name-changed", handler: fun(workspace: neori.Workspace, old_name: string?))
fun(event: "output-mode-changed", handler: fun(output: neori.Output, old_mode: string?))
```

#### `neori.connect(signal, handler)`

Connects a function to a signal: a built-in one, or any other name for a signal of your own.
Returns the function, to disconnect it later.

Parameters:

- `signal` `neori.Signal|string`
- `handler` `F`

Returns `F`.

Can also be called as:

```
fun(signal: "startup", handler: fun()): fun()
fun(signal: "monitor-added", handler: fun(connector: string, output: neori.Output)): fun(connector: string, output: neori.Output)
fun(signal: "monitor-removed", handler: fun(connector: string, output: neori.Output)): fun(connector: string, output: neori.Output)
fun(signal: "lid-open", handler: fun()): fun()
fun(signal: "lid-closed", handler: fun()): fun()
fun(signal: "tablet-mode-on", handler: fun()): fun()
fun(signal: "tablet-mode-off", handler: fun()): fun()
fun(signal: "window-opening", handler: fun(window: neori.Window)): fun(window: neori.Window)
fun(signal: "window-opened", handler: fun(window: neori.Window)): fun(window: neori.Window)
fun(signal: "window-closed", handler: fun(window: neori.Window)): fun(window: neori.Window)
fun(signal: "window-focused", handler: fun(window: neori.Window)): fun(window: neori.Window)
fun(signal: "workspace-focused", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
fun(signal: "workspace-added", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
fun(signal: "workspace-removed", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
fun(signal: "output-focused", handler: fun(output: neori.Output)): fun(output: neori.Output)
fun(signal: "layout-changed", handler: fun(layout: string)): fun(layout: string)
fun(signal: "device-added", handler: fun(device: neori.Device)): fun(device: neori.Device)
fun(signal: "device-removed", handler: fun(device: neori.Device)): fun(device: neori.Device)
fun(signal: "layer-added", handler: fun(layer: neori.LayerSurface)): fun(layer: neori.LayerSurface)
fun(signal: "layer-removed", handler: fun(layer: neori.LayerSurface)): fun(layer: neori.LayerSurface)
fun(signal: "pointer-button", handler: fun(event: neori.PointerButtonEvent)): fun(event: neori.PointerButtonEvent)
fun(signal: "pointer-axis", handler: fun(event: neori.PointerAxisEvent)): fun(event: neori.PointerAxisEvent)
fun(signal: "gesture-swipe-begin", handler: fun(event: neori.GestureBeginEvent)): fun(event: neori.GestureBeginEvent)
fun(signal: "gesture-swipe-update", handler: fun(event: neori.GestureSwipeUpdateEvent)): fun(event: neori.GestureSwipeUpdateEvent)
fun(signal: "gesture-swipe-end", handler: fun(event: neori.GestureEndEvent)): fun(event: neori.GestureEndEvent)
fun(signal: "gesture-pinch-begin", handler: fun(event: neori.GestureBeginEvent)): fun(event: neori.GestureBeginEvent)
fun(signal: "gesture-pinch-update", handler: fun(event: neori.GesturePinchUpdateEvent)): fun(event: neori.GesturePinchUpdateEvent)
fun(signal: "gesture-pinch-end", handler: fun(event: neori.GestureEndEvent)): fun(event: neori.GestureEndEvent)
fun(signal: "gesture-hold-begin", handler: fun(event: neori.GestureBeginEvent)): fun(event: neori.GestureBeginEvent)
fun(signal: "gesture-hold-end", handler: fun(event: neori.GestureEndEvent)): fun(event: neori.GestureEndEvent)
fun(signal: "mode-changed", handler: fun(mode: string?, old_mode: string?)): fun(mode: string?, old_mode: string?)
fun(signal: "config-reloaded", handler: fun()): fun()
fun(signal: "shutdown", handler: fun()): fun()
fun(signal: "window-title-changed", handler: fun(window: neori.Window, old_title: string?)): fun(window: neori.Window, old_title: string?)
fun(signal: "window-fullscreen-changed", handler: fun(window: neori.Window, was_fullscreen: boolean)): fun(window: neori.Window, was_fullscreen: boolean)
fun(signal: "window-floating-changed", handler: fun(window: neori.Window, was_floating: boolean)): fun(window: neori.Window, was_floating: boolean)
fun(signal: "window-urgent-changed", handler: fun(window: neori.Window, was_urgent: boolean)): fun(window: neori.Window, was_urgent: boolean)
fun(signal: "window-workspace-changed", handler: fun(window: neori.Window, old_workspace: neori.WindowWorkspace?)): fun(window: neori.Window, old_workspace: neori.WindowWorkspace?)
fun(signal: "workspace-name-changed", handler: fun(workspace: neori.Workspace, old_name: string?)): fun(workspace: neori.Workspace, old_name: string?)
fun(signal: "output-mode-changed", handler: fun(output: neori.Output, old_mode: string?)): fun(output: neori.Output, old_mode: string?)
```

#### `neori.disconnect(signal, handler?)`

Disconnects a function from a signal, or every function if `handler` is nil. Returns whether
anything was disconnected.

Parameters:

- `signal` `neori.Signal|string`
- `handler?` `function`

Returns `boolean`.

#### `neori.emit(signal, ...)`

Calls every function connected to a signal, right away, with the given arguments.

Parameters:

- `signal` `neori.Signal|string`
- `...` `any`

#### `neori.signals()`

Lists every built-in signal, then every custom signal that has handlers.

Returns `neori.SignalInfo[]`.

### Queries (only return something inside callbacks)

#### `neori.windows()`

Every mapped window.

Returns `neori.Window[]`.

#### `neori.focused_window()`

The focused window.

Returns `neori.Window?`.

#### `neori.window_at(x, y)`

The window at these global logical coordinates, like the one under the pointer:
`neori.window_at(neori.pointer().x, neori.pointer().y)`.

Parameters:

- `x` `number`
- `y` `number`

Returns `neori.Window?`.

#### `neori.outputs()`

Every connected output that is on.

Returns `neori.Output[]`.

#### `neori.focused_output()`

The focused output.

Returns `neori.Output?`.

#### `neori.output_at(x, y)`

The output at these global logical coordinates.

Parameters:

- `x` `number`
- `y` `number`

Returns `neori.Output?`.

#### `neori.workspaces()`

Every workspace, in global index order.

Returns `neori.Workspace[]`.

#### `neori.focused_workspace()`

The focused workspace.

Returns `neori.Workspace?`.

#### `neori.devices()`

Every input device.

Returns `neori.Device[]`.

#### `neori.layers()`

Every layer-shell surface: panels, bars, launchers, notifications, …

Returns `neori.LayerSurface[]`.

#### `neori.pointer()`

The pointer position.

Returns `neori.Pointer?`.

#### `neori.keyboard_layout()`

The name of the active keyboard layout, like `"English (US)"`.

Returns `string?`.

### Config sections

Call one with a table to merge it into the section, or read and assign its fields:

```lua
neori.layout { gaps = 10 }
neori.layout.gaps = 16
print(neori.layout.gaps) --> 16
```

A read returns what the config has set so far, or nil. Changes made after the config loaded,
from a callback or `neori msg lua`, apply to the running config. See the Configuration pages
of the wiki for the fields. Their types, `neori.config.*`, are in neori-config.lua, generated
from the config structs.

#### `neori.input`

Input devices: keyboard, touchpad, mouse, tablet, touch. See the Configuration: Input wiki page.

Type: `neori.config.Input`

#### `neori.layout`

Layout: gaps, focus ring, border, shadow, column widths. See the Configuration: Layout wiki page.

Type: `neori.config.Layout`

#### `neori.cursor`

Cursor theme, size, hiding, shake-to-find.

Type: `neori.config.Cursor`

#### `neori.animations`

Animations. See the Configuration: Animations wiki page.

Type: `neori.config.Animations`

#### `neori.gestures`

Gestures and hot corners. See the Configuration: Gestures wiki page.

Type: `neori.config.Gestures`

#### `neori.switch_events`

Actions for lid and tablet-mode switches. See the Configuration: Switch Events wiki page.

Type: `neori.config.SwitchBinds`

#### `neori.recent_windows`

The recent windows switcher. See the Configuration: Recent Windows wiki page.

Type: `neori.config.RecentWindows`

#### `neori.hotkey_overlay`

The hotkey overlay.

Type: `neori.config.HotkeyOverlay`

#### `neori.overview`

The overview.

Type: `neori.config.Overview`

#### `neori.blur`

Background blur.

Type: `neori.config.Blur`

#### `neori.clipboard`

Clipboard settings.

Type: `neori.config.Clipboard`

#### `neori.config_notification`

The config error notification.

Type: `neori.config.ConfigNotification`

#### `neori.xwayland_satellite`

Xwayland through xwayland-satellite.

Type: `neori.config.XwaylandSatellite`

#### `neori.environment`

Environment variables for programs spawned by neori; `false` unsets a variable.

Type: `neori.config.Environment`

#### `neori.debug`

Debug options.

Type: `neori.config.Debug`

#### `neori.Misc`

The options that aren't sections: call it with a table, or read and assign them.

| field | type | |
|---|---|---|
| `prefer_no_csd` | `boolean?` | Ask apps to not draw their own title bars and borders. |
| `screenshot_path` | `string\|false\|nil` | Where screenshots are saved, as a `strftime` pattern (`~` expands to the home directory). Assigning nil (read back as false) turns saving off. |

Can also be called as:

```
fun(options: { prefer_no_csd?: boolean, screenshot_path?: string|false })
```

#### `neori.misc`

`neori.misc.prefer_no_csd = true`, `neori.misc.screenshot_path = nil`, …

Type: `neori.Misc`

### Rules (neori.rules)

#### `neori.RuleMatch`

What a `where` value matches: a regex (unanchored, like window rules' `match`), a number, a
boolean, a function that gets the value and the object and returns whether it matches, or a
list of these alternatives.

Type: `string|number|boolean|fun(value: any, object: any): boolean|(string|number|boolean|fun(value: any, object: any): boolean)[]`

#### `neori.RuleWhere`

The properties a rule can match. Which ones exist depends on the object: the window keys for
window events, the output keys for `monitor-added`, and so on.

| field | type | |
|---|---|---|
| `id` | `neori.RuleMatch?` | Window, Workspace, LayerSurface. |
| `app_id` | `neori.RuleMatch?` | Window. |
| `title` | `neori.RuleMatch?` | Window. |
| `pid` | `neori.RuleMatch?` | Window, LayerSurface. |
| `focused` | `neori.RuleMatch?` | Window, Output, Workspace. |
| `floating` | `neori.RuleMatch?` | Window. |
| `fullscreen` | `neori.RuleMatch?` | Window. |
| `maximized` | `neori.RuleMatch?` | Window. |
| `urgent` | `neori.RuleMatch?` | Window. |
| `workspace` | `neori.RuleMatch?` | Window: a string matches the workspace's name, a number its index. |
| `output` | `neori.RuleMatch?` | Window, Workspace, LayerSurface, Device (the output it's mapped to): the connector name. |
| `opacity` | `neori.RuleMatch?` | Window. |
| `parent` | `neori.RuleMatch?` | Window: the window it's a dialog of; match it with a function. |
| `name` | `neori.RuleMatch?` | Output, Workspace, Device. |
| `make` | `neori.RuleMatch?` | Output. |
| `model` | `neori.RuleMatch?` | Output. |
| `serial` | `neori.RuleMatch?` | Output. |
| `mode` | `neori.RuleMatch?` | Output: a table; match it with a function. |
| `scale` | `neori.RuleMatch?` | Output. |
| `transform` | `neori.RuleMatch?` | Output. |
| `position` | `neori.RuleMatch?` | Output: a table; match it with a function. |
| `virtual` | `neori.RuleMatch?` | Output. |
| `off` | `neori.RuleMatch?` | Output: whether it's turned off. |
| `vrr_supported` | `neori.RuleMatch?` | Output. |
| `vrr_enabled` | `neori.RuleMatch?` | Output. |
| `idx` | `neori.RuleMatch?` | Workspace. |
| `sysname` | `neori.RuleMatch?` | Device. |
| `product_id` | `neori.RuleMatch?` | Device. |
| `vendor_id` | `neori.RuleMatch?` | Device. |
| `pointer` | `neori.RuleMatch?` | Device. |
| `keyboard` | `neori.RuleMatch?` | Device. |
| `touch` | `neori.RuleMatch?` | Device. |
| `touchpad` | `neori.RuleMatch?` | Device. |
| `namespace` | `neori.RuleMatch?` | LayerSurface. |
| `layer` | `neori.RuleMatch?` | LayerSurface. |
| `keyboard_interactivity` | `neori.RuleMatch?` | LayerSurface. |

#### `neori.RuleSet`

The properties a rule can set, each with the object's setter.

| field | type | |
|---|---|---|
| `floating` | `boolean?` | Window: `float()`, or `open_floating()` before the window opens. |
| `fullscreen` | `boolean?` | Window: `fullscreen()`, or `open_fullscreen()` before the window opens. |
| `maximized` | `boolean?` | Window: `maximize()`, or `open_maximized()` before the window opens. |
| `workspace` | `integer\|string?` | Window: `send_to_workspace()`, or `open_on_workspace()` before the window opens (names only). |
| `output` | `string?` | Window: `send_to_output()`, or `open_on_output()` before the window opens. Workspace: `send_to_output()`. Device: `map_to_output()`. |
| `opacity` | `number?` | Window: `set_opacity()`. |
| `enabled` | `boolean?` | Output: `enable()`. |
| `mode` | `string?` | Output: `set_mode()`. |
| `scale` | `number?` | Output: `set_scale()`. |
| `transform` | `string?` | Output: `set_transform()`. |
| `position` | `neori.Point?` | Output: `set_position()`. |
| `vrr` | `boolean?` | Output: `set_vrr()`. |
| `name` | `string?` | Workspace: `set_name()`. |
| `tap` | `boolean?` | Device: `set_tap()`. |
| `natural_scroll` | `boolean?` | Device: `set_natural_scroll()`. |
| `accel` | `number?` | Device: `set_accel()`. |
| `accel_profile` | `string?` | Device: `set_accel_profile()`. |
| `left_handed` | `boolean?` | Device: `set_left_handed()`. |
| `disabled` | `boolean?` | Device: `set_disabled()`. |
| `scroll_method` | `string?` | Device: `set_scroll_method()`. |
| `click_method` | `string?` | Device: `set_click_method()`. |
| `dwt` | `boolean?` | Device: `set_dwt()`. |
| `middle_emulation` | `boolean?` | Device: `set_middle_emulation()`. |

#### `neori.Rule`

A rule for `neori.rules.add`. It needs `set`, `run` or both.

| field | type | |
|---|---|---|
| `id` | `string?` | Names the rule for `neori.rules.remove`; a number is given when left out. |
| `where` | `neori.RuleWhere?` | Every property must match. |
| `where_any` | `neori.RuleWhere?` | At least one property must match. |
| `where_not` | `neori.RuleWhere?` | Not every property may match: the rule is skipped when all of them do. |
| `where_not_any` | `neori.RuleWhere?` | No property may match: the rule is skipped when any of them does. |
| `set` | `neori.RuleSet?` | Properties to set when the rule matches. |
| `run` | `fun(object: any)?` | Called with the object when the rule matches, after `set`. |
| `when` | `neori.Signal\|neori.Signal[]?` | The events that apply the rule. Without it, the rule is a window rule: `set` keys that work before the window's first frame apply at `window-opening`, and the rest and `run` at `window-opened`. |

#### `neori.rules`

Rules: match windows, outputs, workspaces, devices or layer surfaces when an event fires, and
set their properties or run a function.

```lua
neori.rules.add {
    where = { app_id = "^firefox$" },
    set = { workspace = "web" },
}
neori.rules.add {
    where = { title = "Picture-in-Picture" },
    set = { floating = true, opacity = 0.9 },
    run = function(w) w:center() end,
}
neori.rules.add {
    when = "monitor-added",
    where = { name = "^HDMI-" },
    set = { scale = 1.5 },
}
```

#### `neori.rules.add(rule)`

Adds a rule and returns its id.

Parameters:

- `rule` `neori.Rule`

Returns `string`: id.

#### `neori.rules.remove(rule)`

Removes the rule with this id, or this rule table. Returns whether one was removed.

Parameters:

- `rule` `string|neori.Rule`

Returns `boolean`.

#### `neori.rules.matches(object, rule)`

Whether the object matches the rule's `where` conditions.

Parameters:

- `object` `neori.Window|neori.Output|neori.Workspace|neori.Device|neori.LayerSurface`
- `rule` `neori.Rule`

Returns `boolean`.

#### `neori.rules.apply(object, rule)`

Applies the rule to the object now, if it matches: sets `set` with the setters and calls
`run`. Returns whether it matched.

Parameters:

- `object` `neori.Window|neori.Output|neori.Workspace|neori.Device|neori.LayerSurface`
- `rule` `neori.Rule`

Returns `boolean`.

### Helpers (neori.util)

#### `neori.Rect`

A rectangle, like `w:geometry()` and `out:geometry()` return.

| field | type | |
|---|---|---|
| `x` | `number` |  |
| `y` | `number` |  |
| `width` | `number` |  |
| `height` | `number` |  |

#### `neori.util`

Helpers ported from awesome's `gears`: `neori.util.table`, `.string`, `.math`, `.geometry`,
`.fs` and `neori.util.dump`.

#### `neori.util.dump(data, tag?, depth?)`

Returns a readable, multi-line description of a value and, for a table, its contents, from
awesome's `gears.debug.dump_return`. `print(neori.util.dump(t))` shows it.

Parameters:

- `data` `any`
- `tag?` `string`: A name to show before the value.
- `depth?` `integer`: How deep to go into nested tables; 10 by default.

Returns `string`.

#### `neori.util.table`

Table helpers, from awesome's `gears.table`.

#### `neori.util.table.join(...)`

A new table with the entries of every table: list items are appended, other keys are set,
later tables winning.

Parameters:

- `...` `table?`

Returns `table`.

#### `neori.util.table.crush(target, source, raw?)`

Sets every key of `source` on `target` (with `rawset` if `raw`), and returns `target`.

Parameters:

- `target` `T`
- `source` `table`
- `raw?` `boolean`

Returns `T`.

#### `neori.util.table.from_sparse(t)`

The values with number keys, in key order, as a list without holes.

Parameters:

- `t` `table`

Returns `table`.

#### `neori.util.table.hasitem(t, item)`

The key of `item` in `t`, or nil.

Parameters:

- `t` `table`
- `item` `any`

Returns `any`.

#### `neori.util.table.find_keys(t, matcher, ordered?, max?)`

The keys whose `matcher(key, value)` is true, or nil if there are none.

Parameters:

- `t` `table`
- `matcher` `fun(key: any, value: any): boolean`
- `ordered?` `boolean`: Only look at the list part, in order.
- `max?` `integer`: Stop after this many.

Returns `any[]?`.

#### `neori.util.table.find_first_key(t, matcher, ordered?)`

The first key whose `matcher(key, value)` is true, or nil.

Parameters:

- `t` `table`
- `matcher` `fun(key: any, value: any): boolean`
- `ordered?` `boolean`: Only look at the list part, in order.

Returns `any`.

#### `neori.util.table.keys(t)`

The keys of `t`, sorted.

Parameters:

- `t` `table`

Returns `any[]`.

#### `neori.util.table.count_keys(t)`

The number of keys of `t`, list part included.

Parameters:

- `t` `table`

Returns `integer`.

#### `neori.util.table.keys_filter(t, ...)`

The sorted keys of `t` whose values have one of these types (`"number"`, `"table"`, …).

Parameters:

- `t` `table`
- `...` `string`

Returns `any[]`.

#### `neori.util.table.reverse(t)`

A copy with the list part reversed and the other keys kept.

Parameters:

- `t` `table`

Returns `table`.

#### `neori.util.table.clone(t, deep?)`

A copy of `t`; nested tables are copied too unless `deep` is false.

Parameters:

- `t` `T`
- `deep?` `boolean`

Returns `T`.

#### `neori.util.table.cycle_value(t, value, step_size?, filter?)`

The value `step_size` places after `value` in the list `t` (1 by default, negative goes back),
wrapping around, and its index. With `filter`, values it rejects are skipped.

Parameters:

- `t` `any[]`
- `value` `any`
- `step_size?` `integer`
- `filter?` `fun(value: any): boolean`

Returns:

- `any`: value
- `integer?`: index

#### `neori.util.table.iterate(t, filter, start?)`

An iterator over the items of the list `t` that `filter` accepts, starting at `start` (1 by
default) and wrapping around once.

Parameters:

- `t` `any[]`
- `filter` `fun(item: any): boolean`
- `start?` `integer`

Returns `fun(): any`.

#### `neori.util.table.merge(target, source)`

Appends the list items of `source` to `target`, and returns `target`.

Parameters:

- `target` `T`
- `source` `table`

Returns `T`.

#### `neori.util.table.diff_merge(target, new, identifier, merger?)`

Updates the list `target` to the items of `new`, matching them by `identifier`, and merging
matched items with `merger` if given. Returns `target` and the added, removed and updated items.

Parameters:

- `target` `table`
- `new` `table`
- `identifier` `fun(item: any): any`
- `merger?` `fun(old: any, new: any): any`

Returns:

- `table`: target
- `table`: added
- `table`: removed
- `table`: updated

#### `neori.util.table.map(f, tbl)`

A new table with `f(value)` for every value of `tbl`, at the same keys.

Parameters:

- `f` `fun(value: any): any`
- `tbl` `table`

Returns `table`.

#### `neori.util.string`

String helpers, from awesome's `gears.string`.

#### `neori.util.string.xml_escape(text)`

Escapes `'`, `"`, `<`, `>` and `&` as XML entities, for Pango markup.

Parameters:

- `text` `string`

Returns `string`.

#### `neori.util.string.xml_unescape(text)`

Turns XML entities back into characters.

Parameters:

- `text` `string`

Returns `string`.

#### `neori.util.string.linecount(text)`

The number of lines.

Parameters:

- `text` `string`

Returns `integer`.

#### `neori.util.string.linewrap(text, width?, indent?)`

Wraps lines at `width` characters (72 by default), indenting wrapped lines by `indent` spaces.

Parameters:

- `text` `string`
- `width?` `integer`
- `indent?` `integer`

Returns `string`.

#### `neori.util.string.quote_pattern(s)`

Escapes the characters that Lua patterns treat specially.

Parameters:

- `s` `string`

Returns `string`.

#### `neori.util.string.query_to_pattern(q)`

A Lua pattern that finds `q` literally, ignoring case.

Parameters:

- `q` `string`

Returns `string`.

#### `neori.util.string.split(str, delimiter?)`

Splits at a delimiter character (`"\n"` by default), skipping empty parts between two
delimiters but keeping one at either end.

Parameters:

- `str` `string`
- `delimiter?` `string`

Returns `string[]`.

#### `neori.util.string.psplit(str, pattern?)`

Splits at a Lua pattern (`"\n"` by default); an empty pattern splits into characters.

Parameters:

- `str` `string`
- `pattern?` `string`

Returns `string[]`.

#### `neori.util.string.startswith(str, sub)`

Parameters:

- `str` `string?`
- `sub` `string`

Returns `boolean`.

#### `neori.util.string.endswith(str, sub)`

Parameters:

- `str` `string?`
- `sub` `string`

Returns `boolean`.

#### `neori.util.math`

Math helpers, from awesome's `gears.math`.

#### `neori.util.math.subsets(set)`

Every subset of the list `set`, the empty one first.

Parameters:

- `set` `any[]`

Returns `any[][]`.

#### `neori.util.math.cycle(t, i)`

Wraps the index `i` into 1..`t`, for cycling through a list of `t` items; nil if `t` < 1.

Parameters:

- `t` `integer`
- `i` `integer`

Returns `integer?`.

#### `neori.util.math.round(x)`

Rounds half up to an integer.

Parameters:

- `x` `number`

Returns `integer`.

#### `neori.util.math.sign(x)`

1, -1 or 0.

Parameters:

- `x` `number`

Returns `integer`.

#### `neori.util.geometry`

Rectangle helpers, from awesome's `gears.geometry.rectangle`. A window's `x` and `y` are nil
while it's off screen, so check them before passing its geometry.

#### `neori.util.geometry.square_distance(geom, x, y)`

The squared distance from the point to the nearest pixel of the rectangle; 0 inside it.

Parameters:

- `geom` `neori.Rect`
- `x` `number`
- `y` `number`

Returns `number`.

#### `neori.util.geometry.closest_by_coord(list, x, y)`

The key of the rectangle in `list` closest to the point.

Parameters:

- `list` `table<any, neori.Rect>`
- `x` `number`
- `y` `number`

Returns `any`.

#### `neori.util.geometry.by_coord(list, x, y)`

The key of a rectangle in `list` that contains the point, or nil.

Parameters:

- `list` `table<any, neori.Rect>`
- `x` `number`
- `y` `number`

Returns `any`.

#### `neori.util.geometry.in_direction(dir, list, cur)`

The key of the rectangle in `list` nearest to `cur` in a direction, or nil.

Parameters:

- `dir` `"up"|"down"|"left"|"right"`
- `list` `table<any, neori.Rect>`
- `cur` `neori.Rect`

Returns `any`.

#### `neori.util.geometry.are_equal(a, b)`

Parameters:

- `a` `neori.Rect`
- `b` `neori.Rect`

Returns `boolean`.

#### `neori.util.geometry.is_inside(a, b)`

Whether `a` lies entirely within `b`, edges included.

Parameters:

- `a` `neori.Rect`
- `b` `neori.Rect`

Returns `boolean`.

#### `neori.util.geometry.intersects(a, b)`

Whether the rectangles overlap.

Parameters:

- `a` `neori.Rect`
- `b` `neori.Rect`

Returns `boolean`.

#### `neori.util.geometry.intersection(a, b)`

The overlap of the rectangles, with a width and height of 0 when there is none.

Parameters:

- `a` `neori.Rect`
- `b` `neori.Rect`

Returns `neori.Rect`.

#### `neori.util.geometry.area_remove(areas, elem)`

Removes `elem` from the list of areas, splitting the areas it overlaps into the parts around
it (which can overlap each other). Changes and returns `areas`.

Parameters:

- `areas` `neori.Rect[]`
- `elem` `neori.Rect`

Returns `neori.Rect[]`.

#### `neori.util.fs`

File system helpers, after awesome's `gears.filesystem`. Directories end with `/`.

#### `neori.util.fs.xdg_config_home()`

`$XDG_CONFIG_HOME`, or `~/.config/`.

Returns `string`.

#### `neori.util.fs.xdg_cache_home()`

`$XDG_CACHE_HOME`, or `~/.cache/`.

Returns `string`.

#### `neori.util.fs.xdg_data_home()`

`$XDG_DATA_HOME`, or `~/.local/share/`.

Returns `string`.

#### `neori.util.fs.xdg_data_dirs()`

`$XDG_DATA_DIRS`, or `/usr/local/share/` and `/usr/share/`.

Returns `string[]`.

#### `neori.util.fs.config_dir()`

The directory of the config file.

Returns `string`.

#### `neori.util.fs.cache_dir()`

`neori/` in the cache directory, made if it's missing: a place for scripts to keep files.

Returns `string`.

#### `neori.util.fs.file_readable(path)`

Whether `path` is a file (not a directory) that you can read.

Parameters:

- `path` `string`

Returns `boolean`.

#### `neori.util.fs.file_executable(path)`

Whether `path` is a file (not a directory) that you can run.

Parameters:

- `path` `string`

Returns `boolean`.

#### `neori.util.fs.dir_readable(path)`

Whether `path` is a directory that you can read.

Parameters:

- `path` `string`

Returns `boolean`.

#### `neori.util.fs.dir_writable(path)`

Whether `path` is a directory that you can write to.

Parameters:

- `path` `string`

Returns `boolean`.

#### `neori.util.fs.is_dir(path)`

Parameters:

- `path` `string`

Returns `boolean`.

#### `neori.util.fs.mkdir(dir)`

Makes a directory and its missing parents. Returns true, or false and an error; a directory
that already exists counts as made.

Parameters:

- `dir` `string`

Returns:

- `boolean`: ok
- `string?`: error

#### `neori.prefer_no_csd(prefer)`

Ask apps to not draw their own title bars and borders. Same as `neori.misc.prefer_no_csd`.

Parameters:

- `prefer` `boolean`

#### `neori.screenshot_path(path)`

Where screenshots are saved, as a `strftime` pattern (`~` expands to the home directory), or
false to not save them. Same as `neori.misc.screenshot_path`.

Parameters:

- `path` `string|false`

#### `neori.binds(binds)`

Sets the binds section, as a table of key = action. `neori.bind` is usually easier.

Parameters:

- `binds` `neori.config.Binds`

#### `neori.window_rules(rules)`

Replaces all window rules.

Parameters:

- `rules` `neori.config.WindowRule[]`

#### `neori.layer_rules(rules)`

Replaces all layer rules.

Parameters:

- `rules` `neori.config.LayerRule[]`

#### `neori.scratchpads(scratchpads)`

Replaces all scratchpads.

Parameters:

- `scratchpads` `(string|neori.config.Scratchpad)[]`

#### `neori.window_rule(rule)`

Adds a window rule.

Parameters:

- `rule` `neori.config.WindowRule`

#### `neori.layer_rule(rule)`

Adds a layer rule.

Parameters:

- `rule` `neori.config.LayerRule`

#### `neori.output(name, config)`

Configures an output, by connector name or `"make model serial"`.

Parameters:

- `name` `string`
- `config` `neori.config.Output`

Can also be called as:

```
fun(config: neori.config.Output)
```

#### `neori.workspace(name, config?)`

Declares a named workspace.

Parameters:

- `name` `string`
- `config?` `neori.config.Workspace`

Can also be called as:

```
fun(config: neori.config.Workspace)
```

#### `neori.scratchpad(name, config)`

Declares a named scratchpad.

Parameters:

- `name` `string`
- `config` `neori.config.Scratchpad`

Can also be called as:

```
fun(config: neori.config.Scratchpad)
```

### Environment

#### `neori.setenv(name, value)`

Sets an environment variable for the programs neori spawns from now on, until the config is
reloaded. Only works inside a callback; `neori.environment { … }` sets them in the config.

Parameters:

- `name` `string`
- `value` `string`

#### `neori.unsetenv(name)`

Unsets an environment variable for the programs neori spawns from now on, until the config is
reloaded.

Parameters:

- `name` `string`

#### `neori.getenv(name)`

An environment variable as the programs neori spawns would see it.

Parameters:

- `name` `string`

Returns `string?`.

### Utilities (while the config loads)

#### `neori.is_startup()`

Whether this is the compositor's first config, still starting up: true while it loads and
while its `startup` handlers run; false in a reloaded config.

Returns `boolean`.

#### `neori.set(key, value)`

Writes a value straight into the config data, for a field no section function covers.

Parameters:

- `key` `string`
- `value` `any`

#### `neori.strict(enabled?)`

Turns strict parsing on (unknown config keys become errors) or off. Defaults to on.

Parameters:

- `enabled?` `boolean`

#### `neori.include(...)`

Evaluates other Lua files or directories, relative to the config file.

Parameters:

- `...` `string`

## Window

#### `neori.Window`

A window. Reads return its current state, even for a `Window` kept from an earlier callback;
once the window is closed they return its last known state, no longer focused. Changes apply
right away.

#### `Window:id()`

Stable id, or 0 inside `window-opening`.

Returns `integer`.

#### `Window:app_id()`

Returns `string?`.

#### `Window:title()`

Returns `string?`.

#### `Window:is_focused()`

Returns `boolean`.

#### `Window:is_floating()`

Returns `boolean`.

#### `Window:is_fullscreen()`

Returns `boolean`.

#### `Window:is_urgent()`

Whether the window requests attention.

Returns `boolean`.

#### `Window:is_maximized()`

Whether the window is maximized to the edges of its output.

Returns `boolean`.

#### `Window:pid()`

Process id of the app; nil if unknown, and for X11 windows (they all belong to
xwayland-satellite).

Returns `integer?`.

#### `Window:geometry()`

Where the window is; nil once it's gone.

Returns `neori.WindowGeometry?`.

#### `Window:workspace()`

Returns `neori.WindowWorkspace?`.

#### `Window:output()`

Connector name of its output.

Returns `string?`.

#### `Window:close()`

#### `Window:focus()`

#### `Window:fullscreen(enable?)`

Enters (true) or leaves (false) fullscreen, or toggles it (nil).

Parameters:

- `enable?` `boolean`

#### `Window:maximize(enable?)`

Maximizes the window to the edges of its output (true), restores it (false), or toggles it
(nil).

Parameters:

- `enable?` `boolean`

#### `Window:center()`

Centers the window on its output.

#### `Window:kill()`

Ends the app's process with SIGKILL, for an app that doesn't react to `close()`. Raises an
error for X11 windows, whose process serves every X11 app.

#### `Window:opacity()`

The opacity the window is drawn with, between 0 and 1.

Returns `number`.

#### `Window:set_opacity(opacity)`

Sets the opacity, between 0 and 1; `nil` goes back to the window rules' one.

Parameters:

- `opacity` `number?`

#### `Window:parent()`

The window this one is a dialog or child of.

Returns `neori.Window?`.

#### `Window:begin_move()`

Starts moving the window with the pointer, like Mod+drag, until the mouse button is released.
Needs a held button: call it from a mouse bind or a `pointer-button` handler for a press.

#### `Window:begin_resize(edges?)`

Starts resizing the window with the pointer from `edges`, or without them from the edges
nearest the pointer, until the mouse button is released. Needs a held button, like
`begin_move()`.

Parameters:

- `edges?` `"top"|"bottom"|"left"|"right"|"top-left"|"top-right"|"bottom-left"|"bottom-right"`

#### `Window:float(enable?)`

Moves to the floating (true) or tiling (false) layout, or toggles it (nil).

Parameters:

- `enable?` `boolean`

#### `Window:send_to_workspace(workspace)`

Parameters:

- `workspace` `integer|string`: A global 1-based index or a workspace name.

#### `Window:send_to_output(output)`

Parameters:

- `output` `string`: Connector name.

#### `Window:open_on_output(output)`

Inside `window-opening` only: open on this output.

Parameters:

- `output` `string`

#### `Window:open_on_workspace(workspace)`

Inside `window-opening` only: open on this named workspace.

Parameters:

- `workspace` `string`

#### `Window:open_fullscreen(enable?)`

Inside `window-opening` only: open fullscreen. Defaults to true.

Parameters:

- `enable?` `boolean`

#### `Window:open_maximized(enable?)`

Inside `window-opening` only: open maximized. Defaults to true.

Parameters:

- `enable?` `boolean`

#### `Window:open_floating(enable?)`

Inside `window-opening` only: open floating. Defaults to true.

Parameters:

- `enable?` `boolean`

#### `Window:connect(signal, handler)`

Connects a function to a signal of this window: `"focused"`, `"closed"`, a property signal
(`"title-changed"`, `"fullscreen-changed"`, `"floating-changed"`, `"urgent-changed"`,
`"workspace-changed"`, which also receive the old value), or one of your own. Returns the
function.

Parameters:

- `signal` `"focused"|"closed"|"title-changed"|"fullscreen-changed"|"floating-changed"|"urgent-changed"|"workspace-changed"|string`
- `handler` `F`

Returns `F`.

#### `Window:disconnect(signal, handler?)`

Parameters:

- `signal` `"focused"|"closed"|"title-changed"|"fullscreen-changed"|"floating-changed"|"urgent-changed"|"workspace-changed"|string`
- `handler?` `function`

Returns `boolean`.

#### `Window:emit(signal, ...)`

Calls the functions connected to a signal of this window, with the window and `...`.

Parameters:

- `signal` `string`
- `...` `any`

#### `Window:signals()`

Returns `neori.SignalInfo[]`.

## Output

#### `neori.Output`

An output. Once it's turned off or unplugged, reads return its last known state and
`is_off()` returns true.

#### `Output:name()`

Connector name, like `"eDP-1"`.

Returns `string`.

#### `Output:make()`

Returns `string?`.

#### `Output:model()`

Returns `string?`.

#### `Output:serial()`

Returns `string?`.

#### `Output:mode()`

Current mode, as `"WxH@HZ"`.

Returns `string?`.

#### `Output:modes()`

Returns `string[]`.

#### `Output:scale()`

Returns `number?`.

#### `Output:transform()`

Returns `"normal"|"90"|"180"|"270"|"flipped"|"flipped-90"|"flipped-180"|"flipped-270"|nil`.

#### `Output:position()`

Returns `neori.Point?`.

#### `Output:is_focused()`

Returns `boolean`.

#### `Output:is_virtual()`

Returns `boolean`.

#### `Output:is_off()`

Returns `boolean`.

#### `Output:vrr_supported()`

Returns `boolean`.

#### `Output:vrr_enabled()`

Returns `boolean`.

#### `Output:geometry()`

Position and logical size, in global logical coordinates.

Returns `{ x: integer, y: integer, width: integer, height: integer }?`.

#### `Output:workarea()`

The area left for windows once layer-shell panels and bars reserved their space, in global
logical coordinates.

Returns `{ x: integer, y: integer, width: integer, height: integer }?`.

#### `Output:physical_size()`

Physical size of the screen in millimetres; nil if unknown (like for virtual outputs).

Returns `{ width: integer, height: integer }?`.

#### `Output:windows()`

The windows on this output.

Returns `neori.Window[]`.

#### `Output:on()`

#### `Output:off()`

#### `Output:enable(enable?)`

Turns the output on (true) or off (false), or toggles it (nil).

Parameters:

- `enable?` `boolean`

#### `Output:set_mode(mode)`

Parameters:

- `mode` `string`: `"WxH@HZ"`, `"WxH"`, or `"auto"`.

#### `Output:set_scale(scale)`

Parameters:

- `scale` `number|"auto"`

#### `Output:set_transform(transform)`

Parameters:

- `transform` `"normal"|"90"|"180"|"270"|"flipped"|"flipped-90"|"flipped-180"|"flipped-270"`

#### `Output:set_position(position)`

Parameters:

- `position` `neori.Point|"auto"`

#### `Output:set_vrr(enable, on_demand?)`

Parameters:

- `enable` `boolean`
- `on_demand?` `boolean`

#### `Output:connect(signal, handler)`

Connects a function to a signal of this output: `"focused"`, `"added"`, `"removed"`,
`"mode-changed"` (which also receives the old mode), or one of your own. Output signals are kept by connector name, so they survive a replug.

Parameters:

- `signal` `"focused"|"added"|"removed"|"mode-changed"|string`
- `handler` `F`

Returns `F`.

#### `Output:disconnect(signal, handler?)`

Parameters:

- `signal` `"focused"|"added"|"removed"|"mode-changed"|string`
- `handler?` `function`

Returns `boolean`.

#### `Output:emit(signal, ...)`

Parameters:

- `signal` `string`
- `...` `any`

#### `Output:signals()`

Returns `neori.SignalInfo[]`.

## Workspace

#### `neori.Workspace`

A workspace. Once it's removed, reads return its last known state.

#### `Workspace:idx()`

Global 1-based index.

Returns `integer`.

#### `Workspace:id()`

Stable id.

Returns `integer`.

#### `Workspace:name()`

Returns `string?`.

#### `Workspace:output()`

Connector name of its output.

Returns `string?`.

#### `Workspace:is_focused()`

Returns `boolean`.

#### `Workspace:windows()`

Returns `neori.Window[]`.

#### `Workspace:focus()`

#### `Workspace:send_to_output(output)`

Parameters:

- `output` `string`: Connector name.

#### `Workspace:set_name(name)`

Parameters:

- `name` `string`

#### `Workspace:unset_name()`

#### `Workspace:connect(signal, handler)`

Connects a function to a signal of this workspace: `"focused"`, `"removed"`, `"name-changed"`
(which also receives the old name), or one of your own. Returns the function.

Parameters:

- `signal` `"focused"|"removed"|"name-changed"|string`
- `handler` `F`

Returns `F`.

#### `Workspace:disconnect(signal, handler?)`

Parameters:

- `signal` `"focused"|"removed"|"name-changed"|string`
- `handler?` `function`

Returns `boolean`.

#### `Workspace:emit(signal, ...)`

Parameters:

- `signal` `string`
- `...` `any`

#### `Workspace:signals()`

Returns `neori.SignalInfo[]`.

## Timer

#### `neori.Timer`

A timer made with `neori.after` or `neori.every`.

#### `Timer:start()`

Starts a stopped timer. A running one keeps its countdown.

#### `Timer:again()`

Starts the countdown again from the full timeout, whether the timer was running or not.

#### `Timer:stop()`

#### `Timer:is_running()`

Returns `boolean`.

#### `Timer:timeout()`

The timeout, in milliseconds.

Returns `integer`.

#### `Timer:set_timeout(ms)`

Changes the timeout, from the next `start` or `again`.

Parameters:

- `ms` `integer`

## LayerSurface

#### `neori.LayerSurface`

A layer-shell surface: a panel, bar, launcher, notification, wallpaper, …
Reads return its current state, or its last known one once it's gone.

#### `LayerSurface:id()`

Returns `integer`.

#### `LayerSurface:namespace()`

The namespace the app gave it, like `"waybar"`; layer rules match on it.

Returns `string`.

#### `LayerSurface:layer()`

Returns `"background"|"bottom"|"top"|"overlay"`.

#### `LayerSurface:output()`

Connector name of its output.

Returns `string?`.

#### `LayerSurface:pid()`

Returns `integer?`.

#### `LayerSurface:keyboard_interactivity()`

Returns `"none"|"exclusive"|"on-demand"`.

#### `LayerSurface:close()`

Asks the app to close the surface.

#### `LayerSurface:kill()`

Ends the app's process with SIGKILL.

#### `LayerSurface:connect(signal, handler)`

Connects a function to a signal of this surface: `"removed"`, or one of your own. Its
functions are dropped when it's removed.

Parameters:

- `signal` `"removed"|string`
- `handler` `F`

Returns `F`.

#### `LayerSurface:disconnect(signal, handler?)`

Parameters:

- `signal` `"removed"|string`
- `handler?` `function`

Returns `boolean`.

#### `LayerSurface:emit(signal, ...)`

Parameters:

- `signal` `string`
- `...` `any`

#### `LayerSurface:signals()`

Returns `neori.SignalInfo[]`.

## Device

#### `neori.Device`

An input device. Changes last until the config is reloaded.

#### `Device:name()`

Returns `string`.

#### `Device:sysname()`

Stable per-boot udev name, like `"event5"`.

Returns `string`.

#### `Device:product_id()`

Returns `integer`.

#### `Device:vendor_id()`

Returns `integer`.

#### `Device:is_pointer()`

Returns `boolean`.

#### `Device:is_keyboard()`

Returns `boolean`.

#### `Device:is_touch()`

Returns `boolean`.

#### `Device:is_touchpad()`

Returns `boolean`.

#### `Device:set_tap(enable)`

Parameters:

- `enable` `boolean`

#### `Device:set_natural_scroll(enable)`

Parameters:

- `enable` `boolean`

#### `Device:set_accel(speed)`

Parameters:

- `speed` `number`: From -1.0 to 1.0.

#### `Device:set_left_handed(enable)`

Parameters:

- `enable` `boolean`

#### `Device:set_disabled(disabled)`

Parameters:

- `disabled` `boolean`

#### `Device:set_accel_profile(profile)`

Parameters:

- `profile` `"adaptive"|"flat"`

#### `Device:set_scroll_method(method)`

Parameters:

- `method` `"no-scroll"|"two-finger"|"edge"|"on-button-down"`

#### `Device:set_click_method(method)`

Parameters:

- `method` `"clickfinger"|"button-areas"`

#### `Device:set_dwt(enable)`

Disable-while-typing, for touchpads.

Parameters:

- `enable` `boolean`

#### `Device:set_middle_emulation(enable)`

Emulate a middle click by pressing the left and right buttons together.

Parameters:

- `enable` `boolean`

#### `Device:map_to_output(output)`

Maps a tablet or touch screen to the output with this name, over the config's
`map_to_output`; `nil` goes back to the config's. Kept until the device is removed or the
config is reloaded.

Parameters:

- `output` `string?`

#### `Device:mapped_output()`

The output set with `map_to_output()`, or `nil`.

Returns `string?`.

#### `Device:connect(signal, handler)`

Connects a function to a signal of this device: `"removed"`, or one of your own. Its
functions are dropped when the device is removed.

Parameters:

- `signal` `"removed"|string`
- `handler` `F`

Returns `F`.

#### `Device:disconnect(signal, handler?)`

Parameters:

- `signal` `"removed"|string`
- `handler?` `function`

Returns `boolean`.

#### `Device:emit(signal, ...)`

Parameters:

- `signal` `string`
- `...` `any`

#### `Device:signals()`

Returns `neori.SignalInfo[]`.
