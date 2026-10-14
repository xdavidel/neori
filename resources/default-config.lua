-- Neori default configuration using the neori.* config API.
-- Check the wiki for a full description of the configuration:
-- https://neori-wm.github.io/neori/Configuration:-Introduction

neori.input {
  keyboard = {
    numlock = true,
  },
  touchpad = {
    tap = true,
    natural_scroll = true,
  },
}

neori.layout {
  gaps = 16,
  center_focused_column = "never",
  preset_column_widths = {
    { proportion = 1 / 3 },
    { proportion = 0.5 },
    { proportion = 2 / 3 },
  },
  default_column_width = { proportion = 0.5 },
  focus_ring = {
    width = 4,
    active_color = "#7fc8ff",
    inactive_color = "#505050",
  },
  border = {
    off = true,
    width = 4,
    active_color = "#ffc87f",
    inactive_color = "#505050",
    urgent_color = "#9b0000",
  },
  shadow = {
    softness = 30,
    spread = 5,
    offset = { x = 0, y = 5 },
    color = "#0007",
  },
}

neori.on("startup", neori.spawn("waybar"))

neori.screenshot_path("~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png")

-- Work around WezTerm's initial configure bug by letting windows
-- decide their width for this rule.
neori.window_rule {
  match = { app_id = "^org\\.wezfurlong\\.wezterm$" },
  default_column_width = {},
}

-- Open the Firefox picture-in-picture player as floating by default.
neori.window_rule {
  match = { app_id = "firefox$", title = "^Picture-in-Picture$" },
  open_floating = true,
}

-- Keys consist of modifiers separated by + signs, followed by an XKB key
-- name in the end. "Mod" is a special modifier equal to Super when running
-- on a TTY, and to Alt when running as a winit window.
--
-- Most actions bound here can also be invoked programmatically with
-- `neori msg action do-something`.

-- Mod-Shift-/, which is usually the same as Mod-?, shows important hotkeys.
neori.bind("Mod+Shift+Slash", "show_hotkey_overlay")

-- Suggested binds for running programs: terminal, app launcher.
neori.bind("Mod+Return", neori.spawn("kitty", { hotkey_overlay_title = "Open a Terminal: kitty", ["repeat"] = false }))
neori.bind("Mod+Slash", neori.spawn("fuzzel", { hotkey_overlay_title = "Run a Command: fuzzel" }))
neori.bind("Mod+Alt+Slash", neori.spawn("fuzzel", { hotkey_overlay_title = "Run an Application: fuzzel" }))

-- Example volume keys mappings for PipeWire & WirePlumber.
neori.bind("XF86AudioRaiseVolume", neori.spawn_sh("wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1+ -l 1.0", { allow_when_locked = true }))
neori.bind("XF86AudioLowerVolume", neori.spawn_sh("wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1-", { allow_when_locked = true }))
neori.bind("Mod+Equal", neori.spawn_sh("wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1+ -l 1.0", { allow_when_locked = true, hotkey_overlay_title = "Increase Volume" }))
neori.bind("Mod+Minus", neori.spawn_sh("wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1-", { allow_when_locked = true, hotkey_overlay_title = "Decrease Volume" }))
neori.bind("Mod+M", neori.spawn_sh("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle", { allow_when_locked = true }))
neori.bind("XF86AudioMute", neori.spawn_sh("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle", { allow_when_locked = true }))
neori.bind("XF86AudioMicMute", neori.spawn_sh("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle", { allow_when_locked = true }))

-- Example media keys mapping using playerctl.
neori.bind("XF86AudioPrev", neori.spawn_sh("playerctl previous", { allow_when_locked = true }))
neori.bind("XF86AudioNext", neori.spawn_sh("playerctl next", { allow_when_locked = true }))
neori.bind("XF86AudioPlay", neori.spawn_sh("playerctl play-pause", { allow_when_locked = true }))
neori.bind("XF86AudioPause", neori.spawn_sh("playerctl play-pause", { allow_when_locked = true }))
neori.bind("Mod+p", neori.spawn_sh("playerctl play-pause", { allow_when_locked = true, hotkey_overlay_title = "Play/Pause Song" }))
neori.bind("Mod+Comma", neori.spawn_sh("playerctl previous", { allow_when_locked = true, hotkey_overlay_title = "Previous Song" }))
neori.bind("Mod+Period", neori.spawn_sh("playerctl next", { allow_when_locked = true, hotkey_overlay_title = "Next Song" }))
neori.bind("Mod+Shift+p", neori.spawn_sh("playerctl play-pause", { allow_when_locked = true, hotkey_overlay_title = "Play/Pause" }))
neori.bind("Mod+Ctrl+Shift+p", neori.spawn_sh("playerctl pause", { allow_when_locked = true, hotkey_overlay_title = "Pause All" }))

-- Example brightness key mappings for brightnessctl.
neori.bind("XF86MonBrightnessUp", neori.spawn({ "brightnessctl", "--class=backlight", "set", "+10%" }, { allow_when_locked = true }))
neori.bind("XF86MonBrightnessDown", neori.spawn({ "brightnessctl", "--class=backlight", "set", "10%-" }, { allow_when_locked = true }))

-- Open/close the Overview: a zoomed-out view of workspaces and windows.
neori.bind("Mod+O", { toggle_overview = true, ["repeat"] = false })

-- Go back to the previously focused workspace.
neori.bind("Mod+Tab", { focus_workspace_previous = true, ["repeat"] = false })

neori.bind("Mod+Q", { close_window = true, ["repeat"] = false })

neori.bind("Mod+Left", "focus_column_left")
neori.bind("Mod+Down", "focus_window_down")
neori.bind("Mod+Up", "focus_window_up")
neori.bind("Mod+Right", "focus_column_right")

neori.bind("Mod+Shift+H", "move_column_left")
neori.bind("Mod+Shift+J", "move_window_down")
neori.bind("Mod+Shift+K", "move_window_up")
neori.bind("Mod+Shift+L", "move_column_right")
neori.bind("Mod+Shift+Left", "consume_or_expel_window_left")
neori.bind("Mod+Shift+Down", "move_window_down")
neori.bind("Mod+Shift+Up", "move_window_up")
neori.bind("Mod+Shift+Right", "consume_or_expel_window_right")

neori.bind("Mod+Home", "focus_column_first")
neori.bind("Mod+End", "focus_column_last")
neori.bind("Mod+Ctrl+Home", "move_column_to_first")
neori.bind("Mod+Ctrl+End", "move_column_to_last")

neori.bind("Mod+Alt+Left", "focus_monitor_left")
neori.bind("Mod+Alt+Down", "focus_monitor_down")
neori.bind("Mod+Alt+Up", "focus_monitor_up")
neori.bind("Mod+Alt+Right", "focus_monitor_right")

neori.bind("Mod+Alt+Ctrl+Left", "move_column_to_monitor_left")
neori.bind("Mod+Alt+Ctrl+Down", "move_column_to_monitor_down")
neori.bind("Mod+Alt+Ctrl+Up", "move_column_to_monitor_up")
neori.bind("Mod+Alt+Ctrl+Right", "move_column_to_monitor_right")

neori.bind("Mod+Shift+Ctrl+Left", "move_workspace_to_monitor_left")
neori.bind("Mod+Shift+Ctrl+Down", "move_workspace_to_monitor_down")
neori.bind("Mod+Shift+Ctrl+Up", "move_workspace_to_monitor_up")
neori.bind("Mod+Shift+Ctrl+Right", "move_workspace_to_monitor_right")

neori.bind("Mod+Ctrl+Down", "focus_workspace_down")
neori.bind("Mod+Ctrl+Up", "focus_workspace_up")
neori.bind("Mod+Ctrl+Right", "focus_workspace_down")
neori.bind("Mod+Ctrl+Left", "focus_workspace_up")

neori.bind("Mod+Shift+Page_Down", "move_workspace_down")
neori.bind("Mod+Shift+Page_Up", "move_workspace_up")
neori.bind("Mod+Shift+U", "move_workspace_down")
neori.bind("Mod+Shift+I", "move_workspace_up")

-- You can bind mouse wheel scroll ticks using the following syntax.
-- To avoid scrolling through workspaces really fast, use cooldown_ms.
neori.bind("Mod+WheelScrollDown", { focus_workspace_down = true, cooldown_ms = 150 })
neori.bind("Mod+WheelScrollUp", { focus_workspace_up = true, cooldown_ms = 150 })
neori.bind("Mod+Shift+WheelScrollDown", { move_column_to_workspace_down = true, cooldown_ms = 150 })
neori.bind("Mod+Shift+WheelScrollUp", { move_column_to_workspace_up = true, cooldown_ms = 150 })
neori.bind("Mod+Ctrl+WheelScrollDown", "focus_column_right")
neori.bind("Mod+Ctrl+WheelScrollUp", "focus_column_left")
neori.bind("Mod+Ctrl+Shift+WheelScrollDown", "move_column_right")
neori.bind("Mod+Ctrl+Shift+WheelScrollUp", "move_column_left")

-- Cycle through widths set in preset_column_widths.
neori.bind("Mod+R", "switch_preset_column_width")
neori.bind("Mod+Shift+R", "switch_preset_window_height")
neori.bind("Mod+Ctrl+R", "reset_window_height")

neori.bind("Mod+F", "maximize_column")

-- Expand the focused column to space not taken up by other fully visible columns.
neori.bind("Mod+Alt+F", "expand_column_to_available_width")

neori.bind("Mod+F11", "fullscreen_window")

-- Move the focused window between the floating and the tiling layout.
neori.bind("Mod+Shift+F", "toggle_window_floating")
neori.bind("Mod+Ctrl+F", "switch_focus_between_floating_and_tiling")

-- Toggle tabbed column display mode.
neori.bind("Mod+W", "toggle_column_tabbed_display")

neori.bind("Print", "screenshot")
neori.bind("Ctrl+Print", "screenshot_screen")
neori.bind("Alt+Print", "screenshot_window")

-- It's a good idea to bind an escape hatch to toggle the keyboard shortcuts
-- inhibitor, so a buggy application can't hold your session hostage.
neori.bind("Mod+F10", { toggle_keyboard_shortcuts_inhibit = true, allow_inhibiting = false })

-- The quit action will show a confirmation dialog to avoid accidental exits.
neori.bind("Mod+Shift+E", "quit")
neori.bind("Ctrl+Alt+Delete", "quit")

-- Workspaces are referenced by their global, 1-based index. Mod+1..9 target
-- workspaces 1..9 and Mod+0 targets the 10th; hold Shift to move the focused
-- column along.
for i = 1, 10 do
  local n = i % 10
  local index = (n == 0) and 10 or n
  neori.bind("Mod+" .. n, { focus_workspace = index })
  neori.bind("Mod+Shift+" .. n, { move_column_to_workspace = index })
end
