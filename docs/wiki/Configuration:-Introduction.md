### Per-Section Documentation

You can find documentation for various sections of the config on these wiki pages:

* [`neori.input`](./Configuration:-Input.md)
* [`neori.output`](./Configuration:-Outputs.md)
* [`neori.bind`](./Configuration:-Key-Bindings.md)
* [`neori.switch_events`](./Configuration:-Switch-Events.md)
* [`neori.layout`](./Configuration:-Layout.md)
* [top-level options](./Configuration:-Miscellaneous.md)
* [`neori.window_rule`](./Configuration:-Window-Rules.md)
* [`neori.layer_rule`](./Configuration:-Layer-Rules.md)
* [`neori.animations`](./Configuration:-Animations.md)
* [`neori.gestures`](./Configuration:-Gestures.md)
* [`neori.recent_windows`](./Configuration:-Recent-Windows.md)
* [`neori.debug`](./Configuration:-Debug-Options.md)
* [`neori.include`](./Configuration:-Include.md)

### Loading

Neori will load configuration from `$XDG_CONFIG_HOME/neori/config.lua` or `~/.config/neori/config.lua`, falling back to `/etc/neori/config.lua`.
If both of these files are missing, neori will create `$XDG_CONFIG_HOME/neori/config.lua` with the contents of [the default configuration file](https://gitlab.com/xdavidel/neori/-/blob/main/resources/default-config.lua), which are embedded into the neori binary at build time.
Please use the default configuration file as the starting point for your custom configuration.

The configuration is live-reloaded.
Simply edit and save the config file, and your changes will be applied.
This includes key bindings, output settings like mode, window rules, and everything else.

You can run `neori validate` to parse the config and see any errors.

To use a different config file path, pass it in the `--config` or `-c` argument to `neori`.

You can also set `$NEORI_CONFIG` to the path of the config file.
`--config` always takes precedence.
If `--config` or `$NEORI_CONFIG` doesn't point to a real file, the config will not be loaded.
If `$NEORI_CONFIG` is set to an empty string, it is ignored and the default config location is used instead.

### Syntax

The config is written in [Lua], as a script that calls the `neori` configuration API.
Every setting is a field or a helper call on the global `neori` table, for example `neori.layout { gaps = 16 }` or `neori.bind("Mod+T", "close-window")`.

#### Comments

Lines starting with `--` are comments; they are ignored.
Use `--[[ ... ]]` for block comments:

```lua
--[[
    Everything in here is ignored.
    The display won't be turned off
    as the whole output is commented out.
--]]
neori.output("eDP-1", {
    off = true,
})
```

#### Flags

Toggle options in neori are commonly represented as booleans.
Setting a field to `true` enables it; leaving it out or setting it to `false` disables it.
For example:

```lua
-- "Focus follows mouse" is enabled.
neori.input {
    focus_follows_mouse = true,
}
```

```lua
-- "Focus follows mouse" is disabled.
neori.input {
    focus_follows_mouse = false,
}
```

#### Sections

Most sections can be set more than once; later calls merge into the same settings.
For example:

```lua
-- This is valid: the two input blocks merge.
neori.input {
    keyboard = {},
    -- ...
}

neori.input {
    touchpad = {},
    -- ...
}
```

This is what lets [`neori.include(...)`](./Configuration:-Include.md) work: each included file can adjust the same sections instead of overwriting them.

Sections that configure different devices by name are separate, one entry each:

```lua
-- This is valid: these configure different outputs.
neori.output("eDP-1", {
    -- ...
})

neori.output("HDMI-A-1", {
    -- ...
})
```

### Defaults

Omitting most of the sections of the config file will leave you with the default values for that section.
A notable exception is [`neori.bind`](./Configuration:-Key-Bindings.md): binds do not get filled with defaults, so make sure you do not erase the binds you rely on.

### Breaking Change Policy

As a rule, neori updates should not break existing config files.
(For example, the default config from neori v0.1.0 still parses fine on v25.02 as I'm writing this.)

Exceptions can be made for parsing bugs.
For example, neori used to accept multiple binds to the same key, but this was not intended and did not do anything (the first bind was always used).
A patch release changed neori from silently accepting this to causing a parsing failure.
This is not a blanket rule, I will consider the potential impact of every breaking change like this before deciding to carry on with it.

Keep in mind that the breaking change policy applies only to neori releases.
Commits between releases can and do occasionally break the config as new features are ironed out.
However, I do try to limit these, since several people are running git builds.

[Lua]: https://www.lua.org/
