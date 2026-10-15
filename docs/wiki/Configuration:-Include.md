<sup>Since: 25.11</sup>

You can split your config across multiple files by including them.

```lua
-- Some settings...

neori.include("colors.lua")

-- Some more settings...
```

An included file is an ordinary Lua config file: it can set any sections, add binds, and so on.
Settings from included files are merged with the settings of the file that included them.

An included file can in turn include or require more files.
All included files are watched for changes, and the config live-reloads when any of them change.

### `neori.include(path, ...)`

`neori.include` takes one or more paths and runs each one immediately, in order.

Paths resolve as follows:

* Relative to the directory of the current config file: `colors.lua` or `./colors.lua`
* By absolute path: `/path/to/colors.lua`
* Home directory paths: `~/colors.lua` expands to your home directory

`neori.include` also accepts:

* **Directories**: `neori.include("d.d")` runs every `*.lua` file inside `d.d/`, in sorted order
* **Globs**: `neori.include("conf.d/*.lua")` runs every matching file, in sorted order

```lua
neori.include("~/dotfiles/neori/common.lua")
neori.include("/etc/neori/overrides.lua")
neori.include("conf.d/*.lua")
```

The expanded files are recorded so that they are all watched for changes; editing any of them triggers a config live-reload.

### `require`

Because included files are normal Lua, you can also load them with the standard `require`.
A module searcher looks for `<name>.lua` (or `<name>/init.lua`) relative to the main config's directory:

```lua
require("colors")
```

This runs `colors.lua` the same way `neori.include("colors.lua")` would.

### Merging

Most config sections are *merged*, meaning you can set only a few properties and only those change.
Later calls win over earlier ones for the properties they set.

```lua
-- colors.lua
neori.layout {
    focus_ring = {
        active_color = "blue",
    },
    border = {
        active_color = "green",
    },
}
```

```lua
-- config.lua
neori.include("colors.lua")

neori.layout {
    -- Does not set border or focus-ring colors,
    -- so colors from colors.lua are used.
    gaps = 8,

    border = {
        width = 8,
    },
}
```

Sections that address different things by name are kept separate, one entry each.
`neori.output`, `neori.workspace`, `neori.window_rule`, and `neori.layer_rule` all behave this way, so several files can contribute different outputs and rules.

#### Binds

`neori.bind` records every bind, and the *first* bind wins for a given key.
So if an included file already binds a key, a later `neori.bind` for that same key is rejected as a duplicate.

```lua
-- binds.lua
neori.bind("Mod+T", neori.spawn("alacritty"))
```

```lua
-- config.lua
neori.include("binds.lua")

-- This errors: Mod+T is already bound in binds.lua.
neori.bind("Mod+T", neori.spawn("foot"))
```

To override a bind, include the file that defines it *after* you bind the key, or edit the file directly.

#### Flags

Flags are plain booleans, so they can always be set to `false` to disable them.

```lua
-- csd.lua

-- Write false to explicitly disable.
neori.prefer_no_csd = false
```

```lua
-- config.lua

-- Enable prefer_no_csd in the main config.
neori.prefer_no_csd = true

-- Including csd.lua disables it again.
neori.include("csd.lua")
```

#### Non-merging sections

Some settings represent a single combined structure rather than independently mergeable properties.
Examples are `struts`, the preset width/height lists, individual `animations` subsections, and the pointing device sections in `input`.

```lua
-- struts.lua
neori.layout {
    struts = {
        left = 64,
        right = 64,
    },
}
```

```lua
-- config.lua
neori.layout {
    struts = {
        top = 64,
        bottom = 64,
    },
}

neori.include("struts.lua")

-- Struts are not merged.
-- End result is only left and right struts.
```

### Border and other enable flags

Some subsections have an `off` / `on` style flag.
Because empty tables carry no flag, remember to set the flag explicitly when a section is meant to enable something:

```lua
-- separate.lua
neori.layout {
    border = {
        -- Add this line to enable the border:
        on = true,

        width = 4,
        active_color = "#ffc87f",
        inactive_color = "#505050",
    },
}
```

If you leave out `on = true`, the border subsection has no effect and neori falls back to its default (border disabled).
