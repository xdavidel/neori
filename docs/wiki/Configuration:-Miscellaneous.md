This page documents all top-level options that don't otherwise have dedicated pages.

Here are all of these options at a glance:

```lua
neori.on("startup", neori.spawn("waybar"))
neori.on("startup", neori.spawn("alacritty"))
neori.on("startup", neori.spawn_sh("qs -c ~/source/qs/MyAwesomeShell"))
neori.prefer_no_csd(true)
neori.screenshot_path("~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png")
neori.environment {
    QT_QPA_PLATFORM = "wayland",
    DISPLAY = false,
}
neori.cursor {
    xcursor_theme = "breeze_cursors",
    xcursor_size = 48,
    hide_when_typing = true,
    hide_after_inactive_ms = 1000,
}
neori.overview {
    zoom = 0.5,
    backdrop_color = "#262626",
    workspace_shadow = {
        -- off = true
        softness = 40,
        spread = 10,
        offset = {
            x = 0,
            y = 10,
        },
        color = "#00000050",
    },
}
neori.xwayland_satellite {
    -- off = true
    path = "xwayland-satellite",
}
neori.clipboard {
    disable_primary = true,
}
neori.hotkey_overlay {
    skip_at_startup = true,
    hide_not_bound = true,
}
neori.config_notification {
    disable_failed = true,
}
neori.blur {
    -- off = true
    passes = 3,
    offset = 3.0,
    noise = 0.02,
    saturation = 1.5,
}
```

### Spawning at startup

Add lines like this to spawn processes at neori startup.

`neori.spawn(...)` accepts a path to the program binary as the first argument, followed by arguments to the program.

This option works the same way as the [`spawn` key binding action](./Configuration:-Key-Bindings.md#spawn), so please read about all its subtleties there.

```lua
neori.on("startup", neori.spawn("waybar"))
neori.on("startup", neori.spawn("alacritty"))
```

Note that running neori as a systemd session supports xdg-desktop-autostart out of the box, which may be more convenient to use.
Thanks to this, apps that you configured to autostart in GNOME will also "just work" in neori, without any manual startup configuration.

### Spawning shell commands at startup

<sup>Since: 25.08</sup>

Add lines like this to run shell commands at neori startup.

The argument is a single string that is passed verbatim to `sh`.
You can use shell variables, pipelines, `~` expansion and everything else as expected.

See detailed description in the docs for the [`spawn-sh` key binding action](./Configuration:-Key-Bindings.md#spawn-sh).

```lua
-- Pass all arguments in the same string.
neori.on("startup", neori.spawn_sh("qs -c ~/source/qs/MyAwesomeShell"))
```

### `prefer-no-csd`

This flag will make neori ask the applications to omit their client-side decorations.

If an application will specifically ask for CSD, the request will be honored.
Additionally, clients will be informed that they are tiled, removing some rounded corners.

With `prefer-no-csd` set, applications that negotiate server-side decorations through the xdg-decoration protocol will have focus ring and border drawn around them *without* a solid colored background.

> [!NOTE]
> Unlike most other options, changing `prefer-no-csd` will not entirely affect already running applications.
> It will make some windows rectangular, but won't remove the title bars.
> This mainly has to do with neori working around a [bug in SDL2](https://github.com/libsdl-org/SDL/issues/8173) that prevents SDL2 applications from starting.
>
> Restart applications after changing `prefer-no-csd` in the config to fully apply it.

```lua
neori.prefer_no_csd(true)
```

### `screenshot-path`

Set the path where screenshots are saved.
A `~` at the front will be expanded to the home directory.

The path is formatted with `strftime(3)` to give you the screenshot date and time.

Neori will create the last folder of the path if it doesn't exist.

```lua
neori.screenshot_path("~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png")
```

Pass `false` to disable saving screenshots to disk.

```lua
neori.screenshot_path(false)
```

Both options are also fields of `neori.misc`, which can be read back:

```lua
neori.misc.prefer_no_csd = true
neori.misc.screenshot_path = nil -- don't save screenshots
```

### `environment`

Override environment variables for processes spawned by neori.

```lua
neori.environment {
    QT_QPA_PLATFORM = "wayland",
    -- Unset a variable with false (a nil value can't be stored in a Lua table).
    DISPLAY = false,
}
```

Lua callbacks can also change the environment at runtime with `neori.setenv(name, value)` and
`neori.unsetenv(name)`; see [Lua Scripting](./Lua-Scripting.md#environment).

Note that these variables do not propagate to the systemd global environment, so tools and applications started by systemd do not see them.
In particular, if you start a desktop shell like Dux Modular Shell through systemd, then use its built-in application launcher, the apps won't see these environment variables.

If you want all processes to see the environment variables, you can set them in your login shell config instead (i.e. `~/.bash_profile`).
The `neori-session` shell script runs through the login shell and imports all environment variables to systemd before starting neori.
Keep in mind that all compositors will see variables set in the login shell, not just neori.

### `cursor`

Change the theme and size of the cursor as well as set the `XCURSOR_THEME` and `XCURSOR_SIZE` environment variables.

```lua
neori.cursor {
    xcursor_theme = "breeze_cursors",
    xcursor_size = 48,
}
```

#### `hide-when-typing`

<sup>Since: 0.1.10</sup>

If set, hides the cursor when pressing a key on the keyboard.

> [!NOTE]
> This setting might interfere with games running in Wine in native Wayland mode that use mouselook, such as first-person games.
> If your character's point of view jumps down when you press a key and move the mouse simultaneously, try disabling this setting.

```lua
neori.cursor {
    hide_when_typing = true,
}
```

#### `hide-after-inactive-ms`

<sup>Since: 0.1.10</sup>

If set, the cursor will automatically hide once this number of milliseconds passes since the last cursor movement.

```lua
neori.cursor {
    -- Hide the cursor after one second of inactivity.
    hide_after_inactive_ms = 1000,
}
```

#### `shake-to-find`

<sup>Since: 0.2.0</sup>

Enlarges the cursor when you shake the mouse, to help find it. Set it to `true` to enable it with the defaults, or to a table to tune its appearance, timing, and sensitivity.

```lua
neori.cursor {
    shake_to_find = {
        -- Multiplier applied to `xcursor_size` while enlarged.
        size_multiplier = 4,
        -- Absolute size in px while enlarged; overrides `size_multiplier` when set.
        -- size = 128,
        -- How long the cursor stays enlarged after the last shake.
        timeout_ms = 1000,
        -- Duration of the grow/shrink animation.
        animation_ms = 250,

        -- Sensitivity:
        -- Reversals older than this are forgotten.
        window_ms = 600,
        -- Minimum number of direction reversals within the window to count as a shake.
        min_reversals = 4,
        -- Motion deltas smaller than this many pixels are ignored as jitter.
        min_step = 12,
        -- Minimum direction change to count as a reversal, in degrees.
        reversal_angle_deg = 120,
    },
}
```

Lower `min_reversals`, `min_step`, or `reversal_angle_deg`, or raise `window_ms`, to make the shake easier to trigger.

You can also trigger the enlargement on demand with the `enlarge-cursor` action.

### `overview`

<sup>Since: 25.05</sup>

Settings for the [Overview](./Overview.md).

#### `zoom`

Control how much the workspaces zoom out in the overview.
`zoom` ranges from 0 to 0.75 where lower values make everything smaller.

```lua
-- Make workspaces four times smaller than normal in the overview.
neori.overview {
    zoom = 0.25,
}
```

#### `backdrop-color`

Set the backdrop color behind workspaces in the overview.
The backdrop is also visible between workspaces when switching.

The alpha channel for this color will be ignored.

```lua
-- Make the backdrop light.
neori.overview {
    backdrop_color = "#777777",
}
```

You can also set the color per-output [in the output config](./Configuration:-Outputs.md#backdrop-color).

#### `workspace-shadow`

Control the shadow behind workspaces visible in the overview.

Settings here mirror the normal [`shadow` config in the layout section](./Configuration:-Layout.md#shadow), so check the documentation there.

Workspace shadows are configured for a workspace size normalized to 1080 pixels tall, then zoomed out together with the workspace.
Practically, this means that you'll want bigger spread, offset, and softness compared to window shadows.

```lua
-- Disable workspace shadows in the overview.
neori.overview {
    workspace_shadow = {
        off = true,
    },
}
```

### `xwayland-satellite`

<sup>Since: 25.08</sup>

Settings for integration with [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite).

When a recent enough xwayland-satellite is detected, neori will create the X11 sockets and set `DISPLAY`, then automatically spawn `xwayland-satellite` when an X11 client tries to connect.
If Xwayland dies, neori will keep watching the X11 socket and restart `xwayland-satellite` as needed.
This is very similar to how built-in Xwayland works in other compositors.

`off` disables the integration: neori won't create an X11 socket and won't set the `DISPLAY` environment variable.

`path` sets the path to the `xwayland-satellite` binary.
By default, it's just `xwayland-satellite`, so it's looked up like any other non-absolute program name.

```lua
-- Use a custom build of xwayland-satellite.
neori.xwayland_satellite {
    path = "~/source/rs/xwayland-satellite/target/release/xwayland-satellite",
}
```

### `clipboard`

<sup>Since: 25.02</sup>

Clipboard settings.

Set the `disable-primary` flag to disable the primary clipboard (middle-click paste).
Toggling this flag will only apply to applications started afterward.

```lua
neori.clipboard {
    disable_primary = true,
}
```

### `hotkey-overlay`

Settings for the "Important Hotkeys" overlay.

#### `skip-at-startup`

Set the `skip-at-startup` flag if you don't want to see the hotkey help at neori startup.

```lua
neori.hotkey_overlay {
    skip_at_startup = true,
}
```

#### `hide-not-bound`

<sup>Since: 25.08</sup>

By default, neori will show the most important actions even if they aren't bound to any key, to prevent confusion.
Set the `hide-not-bound` flag if you want to hide all actions not bound to any key.

```lua
neori.hotkey_overlay {
    hide_not_bound = true,
}
```

You can customize which binds the hotkey overlay shows using the [`hotkey-overlay-title` property](./Configuration:-Key-Bindings.md#custom-hotkey-overlay-titles).

### `config-notification`

<sup>Since: 25.08</sup>

Settings for the config created/failed notification.

Set the `disable-failed` flag to disable the "Failed to parse the config file" notification.
For example, if you have a custom one.

```lua
neori.config_notification {
    disable_failed = true,
}
```

### `blur`

<sup>Since: 26.04</sup>

Blur configuration that affects all background blur.

See the [window effects page](./Window-Effects.md) for an overview of background effects.

```lua
-- These are the default values:
neori.blur {
    -- off = true
    passes = 3,
    offset = 3,
    noise = 0.02,
    saturation = 1.5,
}
```

#### `off`

By default, blur is available on request by a window or layer surface (via the `ext-background-effect` protocol).
You can also enable it manually with the `blur true` background effect [window](./Configuration:-Window-Rules.md#background-effect) or [layer](./Configuration:-Layer-Rules.md#background-effect) rule.

Setting the `off` flag will disable all blur, both requested by the window, and configured in window rules.

```lua
neori.blur {
    off = true,
}
```

#### `passes` and `offset`

`passes` controls the number of downsample/upsample passes for dual kawase blur.
More passes produce a larger, smoother blur, but cost more GPU resources.

`offset` is the pixel offset multiplier for each pass.
Offset `1` is the original dual kawase blur.
Larger values produce a smoother blur, at no additional GPU cost.

However, setting `offset` too big will produce visual artifacts.
You will need to increase `passes` to be able to use a bigger `offset` without artifacts.

When configuring blur, try increasing `offset` first (since it doesn't cause any extra GPU load) until you start getting artifacts.
Then, if you still need smoother blur, increase `passes` by 1.
Keep doing this until you get the desired visuals. 

```lua
neori.blur {
    passes = 3,
    offset = 3.0,
}
```

#### `noise`

Amount of noise to add on top of the blur.

This is helpful to reduce color banding artifacts.

```lua
neori.blur {
    noise = 0.02,
}
```

#### `saturation`

Color saturation applied to the blurred background.

Values above `1` increase saturation; values below `1` reduce it.

```lua
neori.blur {
    saturation = 1.5,
}
```
