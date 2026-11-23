---@meta neori

-- Type definitions of neori's Lua API, for lua-language-server (LuaLS).
--
-- Point LuaLS at the directory that holds this file, for example in a `.luarc.json` next to
-- your config:
--
--     { "workspace.library": ["/run/current-system/sw/share/neori/lua"] }
--
-- This file is only read by the language server; neori itself never loads it.

--------------------------------------------------------------------------------
-- Shared types
--------------------------------------------------------------------------------

---A signal that the compositor emits.
---@alias neori.Signal
---| "startup" # The compositor started.
---| "monitor-added" # A monitor was connected.
---| "monitor-removed" # A monitor was disconnected.
---| "lid-open" # The laptop lid was opened.
---| "lid-closed" # The laptop lid was closed.
---| "tablet-mode-on" # Tablet mode was turned on.
---| "tablet-mode-off" # Tablet mode was turned off.
---| "window-opening" # A window is about to open; `w:open_*` methods set where.
---| "window-opened" # A window opened.
---| "window-closed" # A window closed.
---| "window-focused" # A window got keyboard focus.
---| "workspace-focused" # A workspace became the active one.
---| "workspace-added" # A workspace was created.
---| "workspace-removed" # A workspace was removed.
---| "output-focused" # An output became the active one.
---| "layout-changed" # The keyboard layout changed.
---| "device-added" # An input device was connected.
---| "device-removed" # An input device was disconnected.
---| "mode-changed" # A keybinding mode was entered or left.
---| "config-reloaded" # The config was reloaded; runs in the newly loaded config.
---| "shutdown" # The compositor is about to exit.
---| "window-title-changed" # A window's title changed.
---| "window-fullscreen-changed" # A window entered or left fullscreen.
---| "window-floating-changed" # A window moved between the floating and the tiling layout.
---| "window-urgent-changed" # A window started or stopped requesting attention.
---| "window-workspace-changed" # A window moved to another workspace.
---| "workspace-name-changed" # A workspace was named, renamed or unnamed.
---| "output-mode-changed" # An output's mode changed.

---An action: an action name like `"close-window"` (with its arguments, as in
---`"focus-workspace 2"`), or an action table like `{ focus_workspace = 2 }` or the result of
---`neori.spawn`.
---@alias neori.Action string|table

---Options of a bind, the third argument of `neori.bind`.
---@class neori.BindOptions
---Repeat the action while the key is held. Defaults to true, unless the bind has a release
---action.
---@field repeat? boolean
---Ignore the bind for this many milliseconds after it fires.
---@field cooldown_ms? integer
---Also run while the session is locked.
---@field allow_when_locked? boolean
---Let apps inhibit the bind (keyboard shortcuts inhibitor). Defaults to true.
---@field allow_inhibiting? boolean
---Also send the key to the focused app.
---@field pass_through? boolean
---Match the physical key (keycode) instead of the symbol of the current layout.
---@field keycode? boolean
---Title in the hotkey overlay, or `false` to hide the bind from it.
---@field hotkey_overlay_title? string|false

---A bind whose actions run on press and on release.
---@class neori.PressRelease
---@field press? neori.Action|fun()
---@field release? neori.Action|fun()

---A position in the compositor's global logical space.
---@class neori.Point
---@field x number
---@field y number

---@class neori.Pointer: neori.Point
---Connector name of the output under the pointer.
---@field output? string

---Where a window is, in the compositor's global logical space.
---@class neori.WindowGeometry
---Position of the top-left corner; nil while the window isn't on screen.
---@field x number?
---@field y number?
---@field width integer
---@field height integer

---The workspace of a window.
---@class neori.WindowWorkspace
---Global 1-based index.
---@field idx integer
---@field name? string

---A signal, as listed by `neori.signals()` and `obj:signals()`.
---@class neori.SignalInfo
---@field name string
---True for signals that the compositor emits, false for custom ones.
---@field builtin boolean
---How many functions are connected to it.
---@field handlers integer
---What happened (built-in signals only).
---@field description? string
---What the functions receive (built-in signals only).
---@field args? string
---The short name it has on objects, like `"closed"` (`neori.signals()`, for built-in signals
---that reach objects).
---@field object_signal? string
---How many functions are connected to `object_signal`, across all objects.
---@field object_handlers? integer

--------------------------------------------------------------------------------
-- neori
--------------------------------------------------------------------------------

---The neori API.
---@class neori
neori = {}

-- Binds and actions

---Binds a key to an action, a Lua function, or press and release actions.
---
---```lua
---neori.bind("Mod+Q", "close-window")
---neori.bind("Mod+Return", neori.spawn("kitty"), { hotkey_overlay_title = "Terminal" })
---neori.bind("Mod+F1", function() print("pressed") end, { ["repeat"] = false })
---```
---@param key string A key combination, like `"Mod+Shift+E"` or `"Mod+WheelScrollDown"`.
---@param action neori.Action|neori.PressRelease|fun()
---@param options? neori.BindOptions
function neori.bind(key, action, options) end

---Runs an action right away. Only works inside a callback.
---
---```lua
---neori.action("focus-workspace", 2)
---neori.action("spawn", { "kitty", "-e", "htop" })
---```
---@param name string An action name, in kebab or snake case.
---@param args? any The action's argument: a value, a sequence, or a table of fields.
function neori.action(name, args) end

---Builds a `spawn` action that runs a program directly. A trailing table of bind options is
---merged in.
---@param ... string|string[]|neori.BindOptions The command and its arguments.
---@return table
function neori.spawn(...) end

---Builds a `spawn` action that runs a command through `sh -c`.
---@param command string
---@param options? neori.BindOptions
---@return table
function neori.spawn_sh(command, options) end

---Builds a `create-virtual-output` action.
---@param options? { name?: string, width?: integer, height?: integer, refresh_rate?: integer }
---@return table
function neori.create_virtual_output(options) end

---Builds a `remove-virtual-output` action.
---@param name string|{ name: string }
---@return table
function neori.remove_virtual_output(name) end

---Moves the cursor to global logical coordinates. Only works inside a callback.
---@param x number
---@param y number
function neori.warp(x, y) end

-- Keybinding modes

---Declares a keybinding mode: the binds made with `neori.bind` while `declare` runs belong to the
---mode. While the mode is active, they replace the normal binds; give it a bind that leaves it.
---
---```lua
---neori.mode("resize", function()
---    neori.bind("Left", "set-column-width -10%")
---    neori.bind("Escape", "exit-mode")
---end)
---neori.bind("Mod+R", "enter-mode resize")
---```
---@param name string
---@param declare fun()
function neori.mode(name, declare) end

---Enters a keybinding mode, like the `enter-mode` action.
---@param name string
function neori.enter_mode(name) end

---Leaves the active keybinding mode, like the `exit-mode` action.
function neori.exit_mode() end

---The name of the active keybinding mode, or nil.
---@return string?
function neori.current_mode() end

-- Timers

---Calls `callback` once, after `ms` milliseconds. Returns the timer, which the callback also
---receives. Timers made while the config loads start once it has loaded; a config reload
---drops the old config's timers.
---@param ms integer
---@param callback fun(timer: neori.Timer)
---@return neori.Timer
function neori.after(ms, callback) end

---Calls `callback` every `ms` milliseconds, until the timer is stopped.
---@param ms integer At least 1.
---@param callback fun(timer: neori.Timer)
---@return neori.Timer
function neori.every(ms, callback) end

-- Signals

---Connects a function to a built-in event. Like `neori.connect`, but only takes built-in
---events, and also takes a string or table action instead of a function.
---@overload fun(event: "startup", handler: fun()|neori.Action)
---@overload fun(event: "monitor-added", handler: fun(connector: string, output: neori.Output)|neori.Action)
---@overload fun(event: "monitor-removed", handler: fun(connector: string, output: neori.Output)|neori.Action)
---@overload fun(event: "lid-open", handler: fun()|neori.Action)
---@overload fun(event: "lid-closed", handler: fun()|neori.Action)
---@overload fun(event: "tablet-mode-on", handler: fun()|neori.Action)
---@overload fun(event: "tablet-mode-off", handler: fun()|neori.Action)
---@overload fun(event: "window-opening", handler: fun(window: neori.Window))
---@overload fun(event: "window-opened", handler: fun(window: neori.Window))
---@overload fun(event: "window-closed", handler: fun(window: neori.Window))
---@overload fun(event: "window-focused", handler: fun(window: neori.Window))
---@overload fun(event: "workspace-focused", handler: fun(workspace: neori.Workspace))
---@overload fun(event: "workspace-added", handler: fun(workspace: neori.Workspace))
---@overload fun(event: "workspace-removed", handler: fun(workspace: neori.Workspace))
---@overload fun(event: "output-focused", handler: fun(output: neori.Output))
---@overload fun(event: "layout-changed", handler: fun(layout: string))
---@overload fun(event: "device-added", handler: fun(device: neori.Device))
---@overload fun(event: "device-removed", handler: fun(device: neori.Device))
---@overload fun(event: "mode-changed", handler: fun(mode: string?, old_mode: string?))
---@overload fun(event: "config-reloaded", handler: fun()|neori.Action)
---@overload fun(event: "shutdown", handler: fun()|neori.Action)
---@overload fun(event: "window-title-changed", handler: fun(window: neori.Window, old_title: string?))
---@overload fun(event: "window-fullscreen-changed", handler: fun(window: neori.Window, was_fullscreen: boolean))
---@overload fun(event: "window-floating-changed", handler: fun(window: neori.Window, was_floating: boolean))
---@overload fun(event: "window-urgent-changed", handler: fun(window: neori.Window, was_urgent: boolean))
---@overload fun(event: "window-workspace-changed", handler: fun(window: neori.Window, old_workspace: neori.WindowWorkspace?))
---@overload fun(event: "workspace-name-changed", handler: fun(workspace: neori.Workspace, old_name: string?))
---@overload fun(event: "output-mode-changed", handler: fun(output: neori.Output, old_mode: string?))
---@param event neori.Signal
---@param handler function|neori.Action
function neori.on(event, handler) end

---Connects a function to a signal: a built-in one, or any other name for a signal of your own.
---Returns the function, to disconnect it later.
---@generic F: function
---@overload fun(signal: "startup", handler: fun()): fun()
---@overload fun(signal: "monitor-added", handler: fun(connector: string, output: neori.Output)): fun(connector: string, output: neori.Output)
---@overload fun(signal: "monitor-removed", handler: fun(connector: string, output: neori.Output)): fun(connector: string, output: neori.Output)
---@overload fun(signal: "lid-open", handler: fun()): fun()
---@overload fun(signal: "lid-closed", handler: fun()): fun()
---@overload fun(signal: "tablet-mode-on", handler: fun()): fun()
---@overload fun(signal: "tablet-mode-off", handler: fun()): fun()
---@overload fun(signal: "window-opening", handler: fun(window: neori.Window)): fun(window: neori.Window)
---@overload fun(signal: "window-opened", handler: fun(window: neori.Window)): fun(window: neori.Window)
---@overload fun(signal: "window-closed", handler: fun(window: neori.Window)): fun(window: neori.Window)
---@overload fun(signal: "window-focused", handler: fun(window: neori.Window)): fun(window: neori.Window)
---@overload fun(signal: "workspace-focused", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
---@overload fun(signal: "workspace-added", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
---@overload fun(signal: "workspace-removed", handler: fun(workspace: neori.Workspace)): fun(workspace: neori.Workspace)
---@overload fun(signal: "output-focused", handler: fun(output: neori.Output)): fun(output: neori.Output)
---@overload fun(signal: "layout-changed", handler: fun(layout: string)): fun(layout: string)
---@overload fun(signal: "device-added", handler: fun(device: neori.Device)): fun(device: neori.Device)
---@overload fun(signal: "device-removed", handler: fun(device: neori.Device)): fun(device: neori.Device)
---@overload fun(signal: "mode-changed", handler: fun(mode: string?, old_mode: string?)): fun(mode: string?, old_mode: string?)
---@overload fun(signal: "config-reloaded", handler: fun()): fun()
---@overload fun(signal: "shutdown", handler: fun()): fun()
---@overload fun(signal: "window-title-changed", handler: fun(window: neori.Window, old_title: string?)): fun(window: neori.Window, old_title: string?)
---@overload fun(signal: "window-fullscreen-changed", handler: fun(window: neori.Window, was_fullscreen: boolean)): fun(window: neori.Window, was_fullscreen: boolean)
---@overload fun(signal: "window-floating-changed", handler: fun(window: neori.Window, was_floating: boolean)): fun(window: neori.Window, was_floating: boolean)
---@overload fun(signal: "window-urgent-changed", handler: fun(window: neori.Window, was_urgent: boolean)): fun(window: neori.Window, was_urgent: boolean)
---@overload fun(signal: "window-workspace-changed", handler: fun(window: neori.Window, old_workspace: neori.WindowWorkspace?)): fun(window: neori.Window, old_workspace: neori.WindowWorkspace?)
---@overload fun(signal: "workspace-name-changed", handler: fun(workspace: neori.Workspace, old_name: string?)): fun(workspace: neori.Workspace, old_name: string?)
---@overload fun(signal: "output-mode-changed", handler: fun(output: neori.Output, old_mode: string?)): fun(output: neori.Output, old_mode: string?)
---@param signal neori.Signal|string
---@param handler F
---@return F
function neori.connect(signal, handler) end

---Disconnects a function from a signal, or every function if `handler` is nil. Returns whether
---anything was disconnected.
---@param signal neori.Signal|string
---@param handler? function
---@return boolean
function neori.disconnect(signal, handler) end

---Calls every function connected to a signal, right away, with the given arguments.
---@param signal neori.Signal|string
---@param ... any
function neori.emit(signal, ...) end

---Lists every built-in signal, then every custom signal that has handlers.
---@return neori.SignalInfo[]
function neori.signals() end

-- Queries (only return something inside callbacks)

---Every mapped window.
---@return neori.Window[]
function neori.windows() end

---The focused window.
---@return neori.Window?
function neori.focused_window() end

---The window at these global logical coordinates, like the one under the pointer:
---`neori.window_at(neori.pointer().x, neori.pointer().y)`.
---@param x number
---@param y number
---@return neori.Window?
function neori.window_at(x, y) end

---Every connected output that is on.
---@return neori.Output[]
function neori.outputs() end

---The focused output.
---@return neori.Output?
function neori.focused_output() end

---The output at these global logical coordinates.
---@param x number
---@param y number
---@return neori.Output?
function neori.output_at(x, y) end

---Every workspace, in global index order.
---@return neori.Workspace[]
function neori.workspaces() end

---The focused workspace.
---@return neori.Workspace?
function neori.focused_workspace() end

---Every input device.
---@return neori.Device[]
function neori.devices() end

---The pointer position.
---@return neori.Pointer?
function neori.pointer() end

---The name of the active keyboard layout, like `"English (US)"`.
---@return string?
function neori.keyboard_layout() end

-- Config sections. Call one with a table to merge it into the section, or read and assign its
-- fields:
--
--     neori.layout { gaps = 10 }
--     neori.layout.gaps = 16
--     print(neori.layout.gaps) --> 16
--
-- A read returns what the config has set so far, or nil. See the Configuration pages of the
-- wiki for the fields.

---A config section: callable with a table to merge into it, and indexable for its fields.
---@class neori.Section
---@overload fun(config: table)
---@field [string] any

---Input devices: keyboard, touchpad, mouse, tablet, touch. See the Configuration: Input wiki page.
---@type neori.Section
neori.input = {}
---Layout: gaps, focus ring, border, shadow, column widths. See the Configuration: Layout wiki page.
---@type neori.Section
neori.layout = {}
---Cursor theme, size, hiding, shake-to-find.
---@type neori.Section
neori.cursor = {}
---Animations. See the Configuration: Animations wiki page.
---@type neori.Section
neori.animations = {}
---Gestures and hot corners. See the Configuration: Gestures wiki page.
---@type neori.Section
neori.gestures = {}
---Actions for lid and tablet-mode switches. See the Configuration: Switch Events wiki page.
---@type neori.Section
neori.switch_events = {}
---The recent windows switcher. See the Configuration: Recent Windows wiki page.
---@type neori.Section
neori.recent_windows = {}
---The hotkey overlay.
---@type neori.Section
neori.hotkey_overlay = {}
---The overview.
---@type neori.Section
neori.overview = {}
---Background blur.
---@type neori.Section
neori.blur = {}
---Clipboard settings.
---@type neori.Section
neori.clipboard = {}
---The config error notification.
---@type neori.Section
neori.config_notification = {}
---Xwayland through xwayland-satellite.
---@type neori.Section
neori.xwayland_satellite = {}
---Environment variables for programs spawned by neori; `false` unsets a variable.
---@type neori.Section
neori.environment = {}
---Debug options.
---@type neori.Section
neori.debug = {}

---The options that aren't sections: call it with a table, or read and assign them.
---@class neori.Misc
---@overload fun(options: { prefer_no_csd?: boolean, screenshot_path?: string|false })
---Ask apps to not draw their own title bars and borders.
---@field prefer_no_csd boolean?
---Where screenshots are saved, as a `strftime` pattern (`~` expands to the home directory).
---Assigning nil (read back as false) turns saving off.
---@field screenshot_path string|false|nil

---`neori.misc.prefer_no_csd = true`, `neori.misc.screenshot_path = nil`, …
---@type neori.Misc
neori.misc = {}

---Ask apps to not draw their own title bars and borders. Same as `neori.misc.prefer_no_csd`.
---@param prefer boolean
function neori.prefer_no_csd(prefer) end
---Where screenshots are saved, as a `strftime` pattern (`~` expands to the home directory), or
---false to not save them. Same as `neori.misc.screenshot_path`.
---@param path string|false
function neori.screenshot_path(path) end
---Sets the binds section, as a table of key = action. `neori.bind` is usually easier.
---@param binds table
function neori.binds(binds) end
---Replaces all window rules.
---@param rules table[]
function neori.window_rules(rules) end
---Replaces all layer rules.
---@param rules table[]
function neori.layer_rules(rules) end
---Replaces all scratchpads.
---@param scratchpads table[]
function neori.scratchpads(scratchpads) end

---Adds a window rule.
---@param rule table
function neori.window_rule(rule) end

---Adds a layer rule.
---@param rule table
function neori.layer_rule(rule) end

---Configures an output, by connector name or `"make model serial"`.
---@overload fun(config: table)
---@param name string
---@param config table
function neori.output(name, config) end

---Declares a named workspace.
---@overload fun(config: table)
---@param name string
---@param config? table
function neori.workspace(name, config) end

---Declares a named scratchpad.
---@overload fun(config: table)
---@param name string
---@param config table
function neori.scratchpad(name, config) end

-- Environment

---Sets an environment variable for the programs neori spawns from now on, until the config is
---reloaded. Only works inside a callback; `neori.environment { … }` sets them in the config.
---@param name string
---@param value string
function neori.setenv(name, value) end

---Unsets an environment variable for the programs neori spawns from now on, until the config is
---reloaded.
---@param name string
function neori.unsetenv(name) end

---An environment variable as the programs neori spawns would see it.
---@param name string
---@return string?
function neori.getenv(name) end

-- Utilities (while the config loads)

---Whether this is the compositor's first config, still starting up: true while it loads and
---while its `startup` handlers run; false in a reloaded config.
---@return boolean
function neori.is_startup() end

---Writes a value straight into the config data, for a field no section function covers.
---@param key string
---@param value any
function neori.set(key, value) end

---Turns strict parsing on (unknown config keys become errors) or off. Defaults to on.
---@param enabled? boolean
function neori.strict(enabled) end

---Evaluates other Lua files or directories, relative to the config file.
---@param ... string
function neori.include(...) end

--------------------------------------------------------------------------------
-- Window
--------------------------------------------------------------------------------

---A window. Reads return its current state, even for a `Window` kept from an earlier callback;
---once the window is closed they return its last known state. Changes apply right away.
---@class neori.Window
local Window = {}

---Stable id, or 0 inside `window-opening`.
---@return integer
function Window:id() end
---@return string?
function Window:app_id() end
---@return string?
function Window:title() end
---@return boolean
function Window:is_focused() end
---@return boolean
function Window:is_floating() end
---@return boolean
function Window:is_fullscreen() end
---Whether the window requests attention.
---@return boolean
function Window:is_urgent() end
---Whether the window is maximized to the edges of its output.
---@return boolean
function Window:is_maximized() end
---Process id of the app; nil if unknown, and for X11 windows (they all belong to
---xwayland-satellite).
---@return integer?
function Window:pid() end
---Where the window is; nil once it's gone.
---@return neori.WindowGeometry?
function Window:geometry() end
---@return neori.WindowWorkspace?
function Window:workspace() end
---Connector name of its output.
---@return string?
function Window:output() end

function Window:close() end
function Window:focus() end
---Enters (true) or leaves (false) fullscreen, or toggles it (nil).
---@param enable? boolean
function Window:fullscreen(enable) end
---Maximizes the window to the edges of its output (true), restores it (false), or toggles it
---(nil).
---@param enable? boolean
function Window:maximize(enable) end
---Centers the window on its output.
function Window:center() end
---Ends the app's process with SIGKILL, for an app that doesn't react to `close()`. Raises an
---error for X11 windows, whose process serves every X11 app.
function Window:kill() end
---Moves to the floating (true) or tiling (false) layout, or toggles it (nil).
---@param enable? boolean
function Window:float(enable) end
---@param workspace integer|string A global 1-based index or a workspace name.
function Window:send_to_workspace(workspace) end
---@param output string Connector name.
function Window:send_to_output(output) end

---Inside `window-opening` only: open on this output.
---@param output string
function Window:open_on_output(output) end
---Inside `window-opening` only: open on this named workspace.
---@param workspace string
function Window:open_on_workspace(workspace) end
---Inside `window-opening` only: open fullscreen. Defaults to true.
---@param enable? boolean
function Window:open_fullscreen(enable) end
---Inside `window-opening` only: open maximized. Defaults to true.
---@param enable? boolean
function Window:open_maximized(enable) end
---Inside `window-opening` only: open floating. Defaults to true.
---@param enable? boolean
function Window:open_floating(enable) end

---Connects a function to a signal of this window: `"focused"`, `"closed"`, a property signal
---(`"title-changed"`, `"fullscreen-changed"`, `"floating-changed"`, `"urgent-changed"`,
---`"workspace-changed"`, which also receive the old value), or one of your own. Returns the
---function.
---@generic F: function
---@param signal "focused"|"closed"|"title-changed"|"fullscreen-changed"|"floating-changed"|"urgent-changed"|"workspace-changed"|string
---@param handler F
---@return F
function Window:connect(signal, handler) end
---@param signal "focused"|"closed"|"title-changed"|"fullscreen-changed"|"floating-changed"|"urgent-changed"|"workspace-changed"|string
---@param handler? function
---@return boolean
function Window:disconnect(signal, handler) end
---Calls the functions connected to a signal of this window, with the window and `...`.
---@param signal string
---@param ... any
function Window:emit(signal, ...) end
---@return neori.SignalInfo[]
function Window:signals() end

--------------------------------------------------------------------------------
-- Output
--------------------------------------------------------------------------------

---An output. Once it's turned off or unplugged, reads return its last known state and
---`is_off()` returns true.
---@class neori.Output
local Output = {}

---Connector name, like `"eDP-1"`.
---@return string
function Output:name() end
---@return string?
function Output:make() end
---@return string?
function Output:model() end
---@return string?
function Output:serial() end
---Current mode, as `"WxH@HZ"`.
---@return string?
function Output:mode() end
---@return string[]
function Output:modes() end
---@return number?
function Output:scale() end
---@return "normal"|"90"|"180"|"270"|"flipped"|"flipped-90"|"flipped-180"|"flipped-270"|nil
function Output:transform() end
---@return neori.Point?
function Output:position() end
---@return boolean
function Output:is_focused() end
---@return boolean
function Output:is_virtual() end
---@return boolean
function Output:is_off() end
---@return boolean
function Output:vrr_supported() end
---@return boolean
function Output:vrr_enabled() end
---Position and logical size, in global logical coordinates.
---@return { x: integer, y: integer, width: integer, height: integer }?
function Output:geometry() end
---The area left for windows once layer-shell panels and bars reserved their space, in global
---logical coordinates.
---@return { x: integer, y: integer, width: integer, height: integer }?
function Output:workarea() end
---Physical size of the screen in millimetres; nil if unknown (like for virtual outputs).
---@return { width: integer, height: integer }?
function Output:physical_size() end
---The windows on this output.
---@return neori.Window[]
function Output:windows() end

function Output:on() end
function Output:off() end
---Turns the output on (true) or off (false), or toggles it (nil).
---@param enable? boolean
function Output:enable(enable) end
---@param mode string `"WxH@HZ"`, `"WxH"`, or `"auto"`.
function Output:set_mode(mode) end
---@param scale number|"auto"
function Output:set_scale(scale) end
---@param transform "normal"|"90"|"180"|"270"|"flipped"|"flipped-90"|"flipped-180"|"flipped-270"
function Output:set_transform(transform) end
---@param position neori.Point|"auto"
function Output:set_position(position) end
---@param enable boolean
---@param on_demand? boolean
function Output:set_vrr(enable, on_demand) end

---Connects a function to a signal of this output: `"focused"`, `"added"`, `"removed"`,
---`"mode-changed"` (which also receives the old mode), or one of your own. Output signals are kept by connector name, so they survive a replug.
---@generic F: function
---@param signal "focused"|"added"|"removed"|"mode-changed"|string
---@param handler F
---@return F
function Output:connect(signal, handler) end
---@param signal "focused"|"added"|"removed"|"mode-changed"|string
---@param handler? function
---@return boolean
function Output:disconnect(signal, handler) end
---@param signal string
---@param ... any
function Output:emit(signal, ...) end
---@return neori.SignalInfo[]
function Output:signals() end

--------------------------------------------------------------------------------
-- Workspace
--------------------------------------------------------------------------------

---A workspace. Once it's removed, reads return its last known state.
---@class neori.Workspace
local Workspace = {}

---Global 1-based index.
---@return integer
function Workspace:idx() end
---Stable id.
---@return integer
function Workspace:id() end
---@return string?
function Workspace:name() end
---Connector name of its output.
---@return string?
function Workspace:output() end
---@return boolean
function Workspace:is_focused() end
---@return neori.Window[]
function Workspace:windows() end

function Workspace:focus() end
---@param output string Connector name.
function Workspace:send_to_output(output) end
---@param name string
function Workspace:set_name(name) end
function Workspace:unset_name() end

---Connects a function to a signal of this workspace: `"focused"`, `"removed"`, `"name-changed"`
---(which also receives the old name), or one of your own. Returns the function.
---@generic F: function
---@param signal "focused"|"removed"|"name-changed"|string
---@param handler F
---@return F
function Workspace:connect(signal, handler) end
---@param signal "focused"|"removed"|"name-changed"|string
---@param handler? function
---@return boolean
function Workspace:disconnect(signal, handler) end
---@param signal string
---@param ... any
function Workspace:emit(signal, ...) end
---@return neori.SignalInfo[]
function Workspace:signals() end

--------------------------------------------------------------------------------
-- Timer
--------------------------------------------------------------------------------

---A timer made with `neori.after` or `neori.every`.
---@class neori.Timer
local Timer = {}

---Starts a stopped timer. A running one keeps its countdown.
function Timer:start() end
---Starts the countdown again from the full timeout, whether the timer was running or not.
function Timer:again() end
function Timer:stop() end
---@return boolean
function Timer:is_running() end
---The timeout, in milliseconds.
---@return integer
function Timer:timeout() end
---Changes the timeout, from the next `start` or `again`.
---@param ms integer
function Timer:set_timeout(ms) end

--------------------------------------------------------------------------------
-- Device
--------------------------------------------------------------------------------

---An input device. Changes last until the config is reloaded.
---@class neori.Device
local Device = {}

---@return string
function Device:name() end
---Stable per-boot udev name, like `"event5"`.
---@return string
function Device:sysname() end
---@return integer
function Device:product_id() end
---@return integer
function Device:vendor_id() end
---@return boolean
function Device:is_pointer() end
---@return boolean
function Device:is_keyboard() end
---@return boolean
function Device:is_touch() end
---@return boolean
function Device:is_touchpad() end

---@param enable boolean
function Device:set_tap(enable) end
---@param enable boolean
function Device:set_natural_scroll(enable) end
---@param speed number From -1.0 to 1.0.
function Device:set_accel(speed) end
---@param enable boolean
function Device:set_left_handed(enable) end
---@param disabled boolean
function Device:set_disabled(disabled) end
---@param profile "adaptive"|"flat"
function Device:set_accel_profile(profile) end
---@param method "no-scroll"|"two-finger"|"edge"|"on-button-down"
function Device:set_scroll_method(method) end
---@param method "clickfinger"|"button-areas"
function Device:set_click_method(method) end
---Disable-while-typing, for touchpads.
---@param enable boolean
function Device:set_dwt(enable) end
---Emulate a middle click by pressing the left and right buttons together.
---@param enable boolean
function Device:set_middle_emulation(enable) end

---Connects a function to a signal of this device: `"removed"`, or one of your own. Its
---functions are dropped when the device is removed.
---@generic F: function
---@param signal "removed"|string
---@param handler F
---@return F
function Device:connect(signal, handler) end
---@param signal "removed"|string
---@param handler? function
---@return boolean
function Device:disconnect(signal, handler) end
---@param signal string
---@param ... any
function Device:emit(signal, ...) end
---@return neori.SignalInfo[]
function Device:signals() end
