### Overview

Key bindings are declared with `neori.bind(...)` calls in the config.

> [!NOTE]
> This is one of the few sections that *does not* get automatically filled with defaults if you omit it, so make sure to copy it from the default config.

Each bind is a hotkey followed by one action enclosed in curly brackets.
For example:

```lua
neori.bind("Mod+Left", "focus-column-left")
neori.bind("Super+Alt+L", neori.spawn("swaylock"))
```

Binds can also take a Lua function — see [Lua Scripting](./Lua-Scripting.md) for the
runtime API (`neori.bind("key", function() … end)`, `neori.action(name, args)`, Lua event
handlers).

The hotkey consists of modifiers separated by `+` signs, followed by a trigger key in the end.
The trigger key can be either an XKB key name or <sup>Since: next release</sup> a modifier.

Valid modifiers are:

- `Ctrl` or `Control`;
- `Shift`;
- `Alt`;
- `Super` or `Win`;
- `ISO_Level3_Shift` or `Mod5`—this is the AltGr key on certain layouts;
- `ISO_Level5_Shift`: can be used with an xkb lv5 option like `lv5:caps_switch`;
- `Mod`.

`Mod` is a special modifier that is equal to `Super` when running neori on a TTY, and to `Alt` when running neori as a nested winit window.
This way, you can test neori in a window without causing too many conflicts with the host compositor's key bindings.
For this reason, most of the default keys use the `Mod` modifier.

Specific left or right modifier keys like `Alt_L` or `Alt_R` can be used as triggers, but not as modifiers:

```lua
-- Valid:
neori.bind("Ctrl+Alt_L", neori.spawn("alacritty"))

-- Invalid:
-- neori.bind("Alt_L+Ctrl", neori.spawn("alacritty"))

-- Also, this is equivalent to just Alt_L
-- because Alt_L implies the Alt modifier:
neori.bind("Alt+Alt_L", neori.spawn("the Alt here is unnecessary but allowed"))
-- This is the same:
-- neori.bind("Alt_L", neori.spawn("alacritty"))
```

Modifier keys always match by keysym, even though other keys match by keycode by default.

<sup>Since: next release</sup> Generic modifiers like `Shift` or `Alt` can be used as trigger keys.
Binds consisting only of generic modifiers do not depend on the order that you press them:

```lua
-- Will trigger both when pressing Alt then Shift, and Shift then Alt.
neori.bind("Alt+Shift", neori.spawn("alacritty"))

-- The trigger is not a generic modifier, so
-- you need to press it last. Only Ctrl then Left Shift will work.
neori.bind("Ctrl+Shift_L", neori.spawn("alacritty"))
```

<sup>Since: 25.05</sup> You can customize the `Mod` key [in the `input` section of the config](./Configuration:-Input.md#mod-key-mod-key-nested).

> [!TIP]
> To find an XKB name for a particular key, you may use a program like [`wev`](https://git.sr.ht/~sircmpwn/wev).
>
> Open it from a terminal and press the key that you want to detect.
> In the terminal, you will see output like this:
>
> ```
> [14:     wl_keyboard] key: serial: 757775; time: 44940343; key: 113; state: 1 (pressed)
>                       sym: Left         (65361), utf8: ''
> [14:     wl_keyboard] key: serial: 757776; time: 44940432; key: 113; state: 0 (released)
>                       sym: Left         (65361), utf8: ''
> [14:     wl_keyboard] key: serial: 757777; time: 44940753; key: 114; state: 1 (pressed)
>                       sym: Right        (65363), utf8: ''
> [14:     wl_keyboard] key: serial: 757778; time: 44940846; key: 114; state: 0 (released)
>                       sym: Right        (65363), utf8: ''
> ```
>
> Here, look at `sym: Left` and `sym: Right`: these are the key names.
> I was pressing the left and the right arrow in this example.
>
> Keep in mind that binding a key requires spelling out the modifiers that you hold when typing it.
> So, symbols that you type while holding Shift need the Shift modifier in the corresponding binding.
> For example, on the US QWERTY layout, <kbd>&lt;</kbd> is on <kbd>Shift</kbd> + <kbd>,</kbd>, so to bind it, you spell out something like `Mod+Shift+Less` or `Mod+Shift+Comma` (just `Mod+Less` will not work).
>
> As another example, if you've configured the French [BÉPO](https://en.wikipedia.org/wiki/B%C3%89PO) XKB layout, your <kbd>&lt;</kbd> is on <kbd>AltGr</kbd> + <kbd>«</kbd>.
> <kbd>AltGr</kbd> is `ISO_Level3_Shift`, or equivalently `Mod5`, so to bind it, you spell out something like `Mod+Mod5+Less` or `Mod+Mod5+guillemotleft`.
>
> In neori, binds match the physical key (keycode) by default for keys that exist on a US keyboard, so they behave the same on every layout.
> The layout rules below apply to binds with `keycode = false`, and to keys that have no US keycode (like XF86 media keys).
>
> When resolving ASCII (latin) keys, neori will search for the *first* configured XKB layout that has the ASCII key.
> So for example with US Colemak and RU layouts configured, US Colemak will be used for ASCII binds.
>
> Keys from the current layout have priority though.
> For example, if you have US QWERTY and Russian layouts, and both `Mod+Z` and `Mod+Cyrillic_ya` binds, then on Russian the `Mod+Cyrillic_ya` bind will trigger, while on US, the `Mod+Z` bind will trigger.
>
> Also, binds that spell out the typed character have priority over binds that spell out an unshifted character.
> For example, on US QWERTY, `Mod+Shift+At` will have priority over `Mod+Shift+2` if both are configured.
>
> <sup>Since: next release</sup> You used to need to spell out the *unshifted* version of the trigger key.
> So to bind <kbd>&lt;</kbd> on US QWERTY you could use only `Mod+Shift+Comma` (since <kbd>,</kbd> is the unshifted version of that key).
> This is no longer the case: you can spell out either version of the key.

<sup>Since: 0.1.8</sup> Binds will repeat by default (i.e. holding down a bind will make it trigger repeatedly).
You can disable that for specific binds with `repeat=false`:

```lua
neori.bind("Mod+T", neori.spawn("alacritty", { ["repeat"] = false }))
```

Binds can also have a cooldown, which will rate-limit the bind and prevent it from repeatedly triggering too quickly.

```lua
neori.bind("Mod+T", neori.spawn("alacritty", { cooldown_ms = 500 }))
```

This is mostly useful for the scroll bindings.

### Press and Release Bindings

<sup>Since: next release</sup>

Binds can be set to trigger on key press, on key release, or both.
By default, binds trigger on key press.
You can change it using the `press` and `release` fields of a bind.
Each of them takes anything that a bind itself takes: an action, `neori.spawn(...)`, or a Lua function.

```lua
-- Trigger on press (default behavior).
neori.bind("Mod+T", neori.spawn("alacritty"))

-- Trigger on release.
-- Press and release Mod by itself to open the overview.
neori.bind("Mod", { release = "toggle-overview" })

-- Trigger on both press and release with different actions.
--
-- If the press action runs, the release action is guaranteed to run,
-- no matter how you release the keys.
-- You can safely use these binds for things like push-to-talk.
neori.bind("Control_R", {
    press = neori.spawn("notify-send", "Enable push-to-talk"),
    release = neori.spawn("notify-send", "Disable push-to-talk"),
})

-- Release binds trigger only if you don't press anything else in between.
-- So, an Alt+Shift bind like this still lets you hit
-- Alt+Shift+arrows and other shortcuts in applications.
neori.bind("Alt+Shift", { release = neori.spawn("notify-send", "Alt+Shift tapped") })

-- Lua functions work for both press and release.
neori.bind("Mod+Shift+P", {
    press = function() print("pressed") end,
    release = function() print("released") end,
})
```

Release bindings are especially useful when you want to bind a modifier key.
A normal press bind would effectively prevent you from having any other bind containing that modifier.

Release-only binds only trigger when you don't press anything else while holding it.
If you have a release-only `Mod` bind from the example above, and press any other key or mouse button while holding `Mod`, the bind will not trigger (the overview won't open).

In bindings with both press and release, the release action is guaranteed to trigger if the press action triggers.
So, your push-to-talk press-release binding will never get stuck pressed, no matter how you release the keys.

By default, any bind with a release action does not repeat.
You can explicitly set `repeat = true` if needed:

```lua
-- Wouldn't repeat automatically because it has a release action.
neori.bind("Mod+Shift+Q", {
    press = neori.spawn("notify-send", "Pressed (repeating)"),
    release = neori.spawn("notify-send", "Released"),
    ["repeat"] = true,
})
```

### Scroll Bindings

You can bind mouse wheel scroll ticks using the following syntax.
These binds will change direction based on the `natural-scroll` setting.

```lua
neori.bind("Mod+WheelScrollDown", { focus_workspace_down = true, cooldown_ms = 150 })
neori.bind("Mod+WheelScrollUp", { focus_workspace_up = true, cooldown_ms = 150 })
neori.bind("Mod+WheelScrollRight", "focus-column-right")
neori.bind("Mod+WheelScrollLeft", "focus-column-left")
```

Similarly, you can bind touchpad scroll "ticks".
Touchpad scrolling is continuous, so for these binds it is split into discrete intervals based on distance travelled.

These binds are also affected by touchpad's `natural-scroll`, so these example binds are "inverted", since neori has `natural-scroll` enabled for touchpads by default.

```lua
neori.bind("Mod+TouchpadScrollDown", neori.spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.02+"))
neori.bind("Mod+TouchpadScrollUp", neori.spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.02-"))
```

Both mouse wheel and touchpad scroll binds will prevent applications from receiving any scroll events when their modifiers are held down.
For example, if you have a `Mod+WheelScrollDown` bind, then while holding `Mod`, all mouse wheel scrolling will be consumed by neori.

<sup>Since: next release</sup> Mouse wheel bindings are consumed by neori per axis.
If you bind just `Mod+WheelScrollDown` and hold `Mod`, neori will consume vertical scrolling, but not horizontal scrolling.

### Mouse Click Bindings

<sup>Since: 25.01</sup>

You can bind mouse clicks using the following syntax.

```lua
neori.bind("Mod+MouseLeft", "close-window")
neori.bind("Mod+MouseRight", "close-window")
neori.bind("Mod+MouseMiddle", "close-window")
neori.bind("Mod+MouseForward", "close-window")
neori.bind("Mod+MouseBack", "close-window")
```

Mouse clicks operate on the window that was focused at the time of the click, not the window you're clicking.

Note that binding `Mod+MouseLeft` or `Mod+MouseRight` will override the corresponding gesture (moving or resizing the window).

### Tablet Stylus Bindings

<sup>Since: next release</sup>

You can bind tablet stylus buttons using the following syntax.

```lua
neori.bind("Mod+TabletStylusButton1", "close-window")
neori.bind("Mod+TabletStylusButton2", "close-window")
neori.bind("Mod+TabletStylusButton3", "close-window")
```

### Custom Hotkey Overlay Titles

<sup>Since: 25.02</sup>

The hotkey overlay (the Important Hotkeys dialog) shows a hardcoded list of binds.
You can customize this list using the `hotkey-overlay-title` property.

To add a bind to the hotkey overlay, set the property to the title that you want to show:
```lua
neori.bind("Mod+Shift+S", neori.spawn("some-script.sh", { hotkey_overlay_title = "Toggle Dark/Light Style" }))
```

Binds with custom titles are listed after the hardcoded binds and before non-customized Spawn binds.

To remove a hardcoded bind from the hotkey overlay, set the property to null:
```lua
neori.bind("Mod+Q", { close_window = true, hotkey_overlay_title = false })
```

> [!TIP]
> When multiple key combinations are bound to the same action:
> - If any of the binds has a custom hotkey overlay title, neori will show that bind.
> - Otherwise, if any of the binds has a null title, neori will hide the bind.
> - Otherwise, neori will show the first key combination.

Custom titles support [Pango markup](https://docs.gtk.org/Pango/pango_markup.html):

```lua
neori.bind("Mod+Shift+S", neori.spawn("some-script.sh", { hotkey_overlay_title = "<b>Toggle</b> <span foreground='red'>Dark</span>/Light Style" }))
```

![Custom markup example.](https://github.com/user-attachments/assets/2a2ba914-bfa7-4dfa-bb5e-49839034765d)

### Actions

Every action that you can bind is also available for programmatic invocation via `neori msg action`.
Run `neori msg action` to get a full list of actions along with their short descriptions.

Here are a few actions that benefit from more explanation.

#### `spawn`

Run a program.

`spawn` accepts a path to the program binary as the first argument, followed by arguments to the program.
For example:

```lua
-- Run alacritty.
neori.bind("Mod+T", neori.spawn("alacritty"))
-- Run `wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1+`.
neori.bind("XF86AudioRaiseVolume", neori.spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.1+"))
```

> [!TIP]
>
> <sup>Since: 0.1.5</sup>
>
> Spawn bindings have a special `allow-when-locked=true` property that makes them work even while the session is locked:
>
> ```lua
> -- This mute bind will work even when the session is locked.
> neori.bind("XF86AudioMute", neori.spawn("wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle", { allow_when_locked = true }))
> ```

For `spawn`, neori *does not* use a shell to run commands, which means that you need to manually separate arguments.
See [`spawn-sh`](#spawn-sh) below for an action that uses a shell.

```lua
-- Correct: every argument is in its own quotes.
neori.bind("Mod+T", neori.spawn("alacritty", "-e", "/usr/bin/fish"))
-- Wrong: will interpret the whole `alacritty -e /usr/bin/fish` string as the binary path.
neori.bind("Mod+D", neori.spawn("alacritty -e /usr/bin/fish"))
-- Wrong: will pass `-e /usr/bin/fish` as one argument, which alacritty won't understand.
neori.bind("Mod+Q", neori.spawn("alacritty", "-e /usr/bin/fish"))
```

This also means that you cannot expand environment variables or `~`.
If you need this, you can run the command through a shell manually.

```lua
-- Wrong: no shell expansion here. These strings will be passed literally to the program.
neori.bind("Mod+T", neori.spawn("grim", "-o", "$MAIN_OUTPUT", "~/screenshot.png"))
-- Correct: run this through a shell manually so that it can expand the arguments.
-- Note that the entire command is passed as a SINGLE argument,
-- because shell will do its own argument splitting by whitespace.
neori.bind("Mod+D", neori.spawn("sh", "-c", "grim -o $MAIN_OUTPUT ~/screenshot.png"))
-- You can also use a shell to run multiple commands,
-- use pipes, process substitution, and so on.
neori.bind("Mod+Q", neori.spawn("sh", "-c", "notify-send clipboard \"$(wl-paste)\""))
```

As a special case, neori will expand `~` to the home directory *only* at the beginning of the program name.

```lua
-- This will work: one ~ at the very beginning.
neori.bind("Mod+T", neori.spawn("~/scripts/do-something.sh"))
```

#### `spawn-sh`

<sup>Since: 25.08</sup>

Run a command through the shell.

The argument is a single string that is passed verbatim to `sh`.
You can use shell variables, pipelines, `~` expansion, and everything else as expected.

```lua
-- Works with spawn-sh: all arguments in the same string.
neori.bind("Mod+D", neori.spawn_sh("alacritty -e /usr/bin/fish"))
-- Works with spawn-sh: shell variable ($MAIN_OUTPUT), ~ expansion.
neori.bind("Mod+T", neori.spawn_sh("grim -o $MAIN_OUTPUT ~/screenshot.png"))
-- Works with spawn-sh: process substitution.
neori.bind("Mod+Q", neori.spawn_sh("notify-send clipboard \"$(wl-paste)\""))
-- Works with spawn-sh: multiple commands.
neori.bind("Super+Alt+S", neori.spawn_sh("pkill orca || exec orca"))
```

`spawn-sh "some command"` is equivalent to `spawn "sh" "-c" "some command"`—it's just a less confusing shorthand.
Keep in mind that going through the shell incurs a tiny performance penalty compared to directly `spawn`ing some binary.

Using `sh` is hardcoded, consistent with other compositors.
If you want a different shell, write it out using `spawn`, e.g. `spawn "fish" "-c" "some fish command"`.

#### `send-to-scratchpad`, `remove-from-scratchpad`, `toggle-scratchpad`

Scratchpads hide a window away from the normal workspace flow and bring it back on demand.
There is a default scratchpad, plus any number of named ones:

```lua
-- Hide the focused window in the default scratchpad.
neori.bind("Mod+Alt+minus", "send-to-scratchpad")
-- Bring it back, shown centered on the active workspace.
neori.bind("Mod+Alt+plus", "remove-from-scratchpad")
-- Show it if hidden, hide it if shown.
neori.bind("Mod+Alt+Return", "toggle-scratchpad")

-- The same three actions, for the scratchpad named "term":
neori.bind("Mod+Ctrl+m", "send-to-scratchpad term")
neori.bind("Mod+Ctrl+u", { remove_from_scratchpad = "term" })
neori.bind("Mod+Ctrl+t", { toggle_scratchpad = "term" })
```

Declare a scratchpad in the config to give it a spawn command:

```lua
-- The default scratchpad.
neori.scratchpad { spawn = { "kitty", "--class", "neori-scratch" } }
-- Named scratchpads. `spawn_sh` runs the command through sh instead.
neori.scratchpad("term", { spawn_sh = "alacritty --class neori-term" })
-- No spawn: send a window to it first with send-to-scratchpad.
neori.scratchpad("browser")
```

Toggling a scratchpad that is empty and has a spawn command runs the command and adopts the
new window into that scratchpad, so the first press opens the window. Toggling again hides it,
and toggling once more shows it again — the command is only re-run when the scratchpad has
become empty again (for example after the window was closed or removed).

#### `quit`

Exit neori after showing a confirmation dialog to avoid accidentally triggering it.

```lua
neori.bind("Mod+Shift+E", "quit")
```

If you want to skip the confirmation dialog, set the flag like so:

```lua
neori.bind("Mod+Shift+E", { quit = true, skip_confirmation = true })
```

#### `do-screen-transition`

<sup>Since: 0.1.6</sup>

Freeze the screen for a brief moment then crossfade to the new contents.

```lua
neori.bind("Mod+Return", "do-screen-transition")
```

This action is mainly useful to trigger from scripts changing the system theme or style (between light and dark for example).
It makes transitions like this, where windows change their style one by one, look smooth and synchronized.

For example, using the GNOME color scheme setting:

```shell
neori msg action do-screen-transition
dconf write /org/gnome/desktop/interface/color-scheme "\"prefer-dark\""
```

By default, the screen is frozen for 250 ms to give windows time to redraw, before the crossfade.
You can set this delay like this:

```lua
neori.bind("Mod+Return", { do_screen_transition = true, delay_ms = 100 })
```

Or, in scripts:

```shell
neori msg action do-screen-transition --delay-ms 100
```

#### `toggle-window-rule-opacity`

<sup>Since: 25.02</sup>

Toggle the opacity window rule of the focused window.
This only has an effect if the window's opacity window rule is already set to semitransparent.

```lua
neori.bind("Mod+O", "toggle-window-rule-opacity")
```

#### `screenshot`, `screenshot-screen`, `screenshot-window`

Actions for taking screenshots.

- `screenshot`: opens the built-in interactive screenshot UI.
- `screenshot-screen`, `screenshot-window`: takes a screenshot of the focused screen or window respectively.

The screenshot is both stored to the clipboard and saved to disk, according to the [`screenshot-path` option](./Configuration:-Miscellaneous.md#screenshot-path).

<sup>Since: 25.02</sup> You can disable saving to disk for a specific bind with the `write-to-disk=false` property:

```lua
neori.bind("Ctrl+Print", { screenshot_screen = true, write_to_disk = false })
neori.bind("Alt+Print", { screenshot_window = true, write_to_disk = false })
```

In the interactive screenshot UI, pressing <kbd>Ctrl</kbd><kbd>C</kbd> will copy the screenshot to the clipboard without writing it to disk.

<sup>Since: 25.05</sup> You can hide the mouse pointer in screenshots with the `show-pointer=false` property:

```lua
-- The pointer will be hidden by default
-- (you can still show it by pressing P).
neori.bind("Print", { screenshot = true, show_pointer = false })
-- The pointer will be hidden on the screenshot.
neori.bind("Ctrl+Print", { screenshot_screen = true, show_pointer = false })
```

<sup>Since: 26.04</sup> You can show the mouse pointer on window screenshots with the `show-pointer=true` property.
The pointer will be included only if the window is currently receiving pointer input (usually this means the pointer is on top of the window).

```lua
-- The pointer will be visible on the screenshot
-- if it's on top of the window.
neori.bind("Alt+Print", { screenshot_window = true, show_pointer = true })
```

#### `toggle-keyboard-shortcuts-inhibit`

<sup>Since: 25.02</sup>

Applications such as remote-desktop clients and software KVM switches may request that neori stops processing its keyboard shortcuts so that they may, for example, forward the key presses as-is to a remote machine.
`toggle-keyboard-shortcuts-inhibit` is an escape hatch that toggles the inhibitor.
It's a good idea to bind it, so a buggy application can't hold your session hostage.

```lua
neori.bind("Mod+Escape", "toggle-keyboard-shortcuts-inhibit")
```

You can also make certain binds ignore inhibiting with the `allow-inhibiting=false` property.
They will always be handled by neori and never passed to the window.

```lua
-- This bind will always work, even when using a virtual machine.
neori.bind("Super+Alt+L", neori.spawn("swaylock", { allow_inhibiting = false }))
```

#### `toggle-touchpad`

<sup>Since: next release</sup>

Toggle the touchpad on or off at runtime.
This provides a quick way to disable the touchpad without editing the config file.

```lua
neori.bind("Mod+F9", "toggle-touchpad")
```

The toggle state is combined with the [`off` setting](./Configuration:-Input.md#pointing-devices) in the config: both the config and the toggle must allow the touchpad for it to be enabled.
The toggle state resets when neori restarts, and applies to newly hot-plugged touchpads.
