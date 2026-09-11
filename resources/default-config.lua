-- Neori default configuration using the neori.* config API.
-- Check the wiki for a full description of the configuration:
-- https://neori-wm.github.io/neori/Configuration:-Introduction
--
-- This file is written so that an EMPTY config behaves identically to this one
-- when the active settings are removed. The commented examples below show every
-- option neori supports — uncomment and tweak the ones you want.

--------------------------------------------------------------------------------
-- Input
--------------------------------------------------------------------------------
--
-- See https://neori-wm.github.io/neori/Configuration:-Input for the full
-- description of every option.

neori.input {
  keyboard = {
    numlock = true,
    -- xkb = { layout = "us,ru", options = "grp:win_space_toggle,compose:ralt" },
    -- repeat_delay = 600,
    -- repeat_rate = 25,
    -- track_layout = "global",            -- or "window"
  },
  touchpad = {
    tap = true,
    natural_scroll = true,
    -- off = false,
    -- dwt = false,                         -- disable-while-typing
    -- dwtp = false,                        -- disable-while-trackpointing
    -- drag = true,
    -- drag_lock = false,
    -- click_method = "clickfinger",        -- or "button_areas"
    -- accel_speed = 0.0,                   -- range -1..1
    -- accel_profile = "adaptive",          -- or "flat"
    -- scroll_method = "two_finger",        -- "two_finger"|"edge"|"on_button_down"|"no_scroll"
    -- scroll_button = 273,                 -- BTN_RIGHT
    -- scroll_button_lock = false,
    -- tap_button_map = "left_right_middle", -- or "left_middle_right"
    -- left_handed = false,
    -- disabled_on_external_mouse = false,
    -- middle_emulation = false,
    -- scroll_factor = { base = 1.0, horizontal = 1.0, vertical = 1.0 },
    -- pinch_sensitivity = 1.0,
  },
  -- mouse = {
  --   off = false,
  --   natural_scroll = false,
  --   accel_speed = 0.0,
  --   accel_profile = "adaptive",
  --   middle_emulation = false,
  --   scroll_factor = { base = 1.0 },
  -- },
  -- trackpoint = {
  --   off = false,
  --   natural_scroll = false,
  --   accel_speed = 0.0,
  --   accel_profile = "adaptive",
  -- },
  -- trackball = {
  --   off = false,
  --   accel_speed = 0.0,
  -- },
  -- tablet = {
  --   map_to_output = "eDP-1",
  --   map_to_focused_output = false,
  --   map_to_focused_window = false,
  --   left_handed = false,
  --   -- calibration_matrix = { 1.0, 0.0, 0.0,  0.0, 1.0, 0.0 },
  -- },
  -- touch = {
  --   map_to_output = "eDP-1",
  -- },
  -- disable_power_key_handling = false,
  -- focus_follows_mouse = { max_scroll_amount = "0%" },
  -- warp_mouse_to_focus = { mode = "center_xy" },   -- or "center_xy_always"
  -- workspace_auto_back_and_forth = false,
  -- mod_key = "super",                     -- "ctrl"|"shift"|"alt"|"super"|"iso_level3_shift"|"iso_level5_shift"
  -- mod_key_nested = "alt",                -- override when running nested in winit
}

--------------------------------------------------------------------------------
-- Layout
--------------------------------------------------------------------------------

neori.layout {
  gaps = 16,
  center_focused_column = "never",         -- "never" | "always" | "on_overflow"
  preset_column_widths = {
    { proportion = 1 / 3 },
    { proportion = 0.5 },
    { proportion = 2 / 3 },
  },
  default_column_width = { proportion = 0.5 },
  -- preset_window_heights = { { proportion = 1/3 }, { proportion = 0.5 }, { proportion = 2/3 } },
  -- always_center_single_column = false,
  -- empty_workspace_above_first = false,
  -- default_column_display = "normal",    -- or "tabbed"
  -- background_color = "#404040",
  -- struts = { left = 0, right = 0, top = 0, bottom = 0 },

  focus_ring = {
    width = 4,
    active_color = "#7fc8ff",
    inactive_color = "#505050",
    -- off = false,
    -- urgent_color = "#9b0000",
    -- active_gradient = { from = "#80c8ff", to = "#bbddff", angle = 45 },
    -- inactive_gradient = { from = "#505050", to = "#303030", angle = 90 },
    -- urgent_gradient = { from = "#9b0000", to = "#550000", angle = 90 },
  },

  border = {
    off = true,
    width = 4,
    active_color = "#ffc87f",
    inactive_color = "#505050",
    urgent_color = "#9b0000",
    -- active_gradient = { from = "#ffc87f", to = "#ff8c00", angle = 45 },
  },

  shadow = {
    softness = 30,
    spread = 5,
    offset = { x = 0, y = 5 },
    color = "#0007",
    -- on = false,
    -- draw_behind_window = false,
    -- inactive_color = "#0004",
  },

  -- tab_indicator = {
  --   hide_when_single_tab = true,
  --   gap = 5,
  --   width = 4,
  --   length = { total_proportion = 0.5 },
  --   position = "left",                 -- "left"|"right"|"top"|"bottom"
  --   gaps_between_tabs = 0,
  --   corner_radius = 0,
  --   active_color = "#7fc8ff",
  --   inactive_color = "#505050",
  -- },

  -- insert_hint = {
  --   color = "#7fc8ff80",
  --   -- gradient = { from = "#7fc8ff80", to = "#bbddff80", angle = 45 },
  -- },
}

--------------------------------------------------------------------------------
-- Appearance: animations, blur
--------------------------------------------------------------------------------

-- neori.animations {
--   slowdown = 1.0,
--   workspace_switch = { kind = { spring = { damping_ratio = 1.0, stiffness = 1000, epsilon = 0.0001 } } },
--   window_open = { duration_ms = 150, curve = "ease_out_expo" },
--   window_close = { duration_ms = 150, curve = "ease_out_quad" },
--   window_movement = { kind = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.0001 } } },
--   window_resize = { kind = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.0001 } } },
--   horizontal_view_movement = { kind = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.0001 } } },
--   config_notification_open_close = { kind = { spring = { damping_ratio = 0.6, stiffness = 1000, epsilon = 0.001 } } },
--   exit_confirmation_open_close = { kind = { spring = { damping_ratio = 0.6, stiffness = 500, epsilon = 0.01 } } },
--   screenshot_ui_open = { duration_ms = 200, curve = "ease_out_quad" },
--   overview_open_close = { kind = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.0001 } } },
--   recent_windows_close = { kind = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.001 } } },
--   -- off = true,                       -- disable all animations
-- }

-- neori.blur {
--   passes = 3,
--   offset = 3.0,
--   noise = 0.02,
--   saturation = 1.5,
--   -- off = true,
-- }

--------------------------------------------------------------------------------
-- Cursor
--------------------------------------------------------------------------------

-- neori.cursor {
--   xcursor_theme = "default",
--   xcursor_size = 24,
--   hide_when_typing = false,
--   hide_after_inactive_ms = 5000,
--   shake_to_find = {
--     enable = true,
--     size_multiplier = 4.0,
--     -- size = 96,                        -- explicit pixel size (overrides multiplier)
--     timeout_ms = 1000,
--     animation_ms = 250,
--     window_ms = 600,
--     min_reversals = 4,
--     min_step = 12.0,
--     reversal_angle_deg = 120.0,
--   },
-- }

--------------------------------------------------------------------------------
-- Clipboard, config notification, hotkey overlay
--------------------------------------------------------------------------------

-- neori.clipboard {
--   disable_primary = false,               -- true: drop PRIMARY selection, keep CLIPBOARD only
-- }

-- neori.config_notification {
--   disable_failed = false,                -- true: don't flash the notification on config errors
-- }

-- neori.hotkey_overlay {
--   skip_at_startup = false,
--   hide_not_bound = false,                -- only show binds you've actually set
-- }

--------------------------------------------------------------------------------
-- Overview, gestures
--------------------------------------------------------------------------------

-- neori.overview {
--   zoom = 0.5,
--   backdrop_color = "#262626",
--   workspace_shadow = {
--     offset = { x = 0, y = 10 },
--     softness = 40,
--     spread = 10,
--     color = "#00000050",
--     -- off = false,
--   },
-- }

-- neori.gestures {
--   hot_corners = {
--     -- off = false,
--     top_left = false,                     -- open overview
--     top_right = false,
--     bottom_left = false,
--     bottom_right = false,
--   },
--   dnd_edge_view_scroll = { trigger_width = 30, delay_ms = 100, max_speed = 1500 },
--   dnd_edge_workspace_switch = { trigger_height = 50, delay_ms = 100, max_speed = 1500 },
-- }

--------------------------------------------------------------------------------
-- Recent (most-recently-used) window switcher
--------------------------------------------------------------------------------

-- neori.recent_windows {
--   on = true,
--   debounce_ms = 750,
--   open_delay_ms = 150,
--   highlight = {
--     active_color = "#999999",
--     urgent_color = "#ff9999",
--     padding = 30,
--     corner_radius = 0,
--   },
--   previews = { max_height = 480, max_scale = 0.5 },
-- }

--------------------------------------------------------------------------------
-- Xwayland satellite, environment, prefer-no-csd, screenshot path
--------------------------------------------------------------------------------

-- neori.xwayland_satellite {
--   -- off = true,                         -- disable the Xwayland shim entirely
--   path = "xwayland-satellite",
-- }

-- neori.environment {
--   "DISPLAY",
--   "XDG_CURRENT_DESKTOP=neori",
--   "QT_QPA_PLATFORM=wayland",
--   "MOZ_ENABLE_WAYLAND=1",
--   "SDL_VIDEODRIVER=wayland",
-- }

-- neori.prefer_no_csd = true                -- ask toplevels to hide their own title bars

neori.screenshot_path("~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png")

--------------------------------------------------------------------------------
-- Debug (advanced, usually safe to leave off)
--------------------------------------------------------------------------------

-- neori.debug {
--   dbus_interfaces_in_non_session_instances = false,
--   wait_for_frame_completion_before_queueing = false,
--   enable_overlay_planes = false,
--   disable_cursor_plane = false,
--   disable_direct_scanout = false,
--   force_disable_connectors_on_resume = false,
--   -- render_drm_device = "/dev/dri/renderD128",
--   -- ignored_drm_devices = { "/dev/dri/renderD129" },
--   force_pipewire_invalid_modifier = false,
--   emulate_zero_presentation_time = false,
--   disable_resize_throttling = false,
--   disable_transactions = false,
--   keep_laptop_panel_on_when_lid_is_closed = false,
--   disable_monitor_names = false,
--   skip_cursor_only_updates_during_vrr = false,
--   disable_10bit_output = false,
--   disable_pipewire_dmabuf = false,
-- }

--------------------------------------------------------------------------------
-- Outputs
--------------------------------------------------------------------------------
--
-- Configure physical outputs by connector name (e.g. "eDP-1", "DP-3"), or
-- declare a virtual (headless) one with `create_virtual = true`. Virtual outputs
-- are useful for Sunshine / Moonlight streaming and for headless workstations;
-- see https://neori-wm.github.io/neori/Virtual-Outputs.

-- neori.output("eDP-1", {
--   -- off = false,
--   mode = "1920x1080@120.030",
--   scale = 2.0,
--   transform = "normal",                 -- or "90"|"180"|"270"|"flipped"|"flipped-90"|...
--   position = { x = 0, y = 0 },
--   focus_at_startup = true,
--   variable_refresh_rate = true,         -- true | { on_demand = true }
--   -- max_bpc = 10,
--   -- background_color = "#001100",
--   -- backdrop_color = "#000000",
--   -- hot_corners = { top_left = true },
--   -- modeline = { ... },
--   -- layout = { background_color = "#111", ...per-output overrides... },
-- })

-- neori.output("sunshine", {
--   create_virtual = true,                -- virtual output created at startup / reload
--   mode = "1920x1080@144.000",
-- })

--------------------------------------------------------------------------------
-- Named workspaces
--------------------------------------------------------------------------------

-- neori.workspace("browser", { open_on_output = "eDP-1" })
-- neori.workspace("chat")

---------------------------------------------------------------------------------
-- Scratchpads
---------------------------------------------------------------------------------

-- Named scratchpads, each holding at most one window. Actions address them by name:
--   neori.bind("Mod+Alt+m", "send-to-scratchpad term")
--   neori.bind("Mod+Alt+Return", "toggle-scratchpad term")
--   neori.bind("Mod+Alt+plus", { remove_from_scratchpad = "term" })
-- An action without a name uses the default scratchpad.
--
-- Give a scratchpad a spawn command and the first toggle-while-empty runs it, adopting the
-- new window into that scratchpad; toggling it away hides the window, toggling again brings
-- it back (the command is re-run only once the scratchpad is empty again).

-- neori.scratchpad { spawn = { "kitty", "--class", "neori-scratch" } }  -- default scratchpad
-- neori.scratchpad("term", { spawn_sh = "alacritty --class neori-term" })
-- neori.scratchpad("browser")  -- no spawn: send a window to it first with send-to-scratchpad

--------------------------------------------------------------------------------
-- Window rules
--------------------------------------------------------------------------------

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

-- Full set of per-rule knobs (see wiki Configuration:-Window-Rules):
--
-- neori.window_rule {
--   matches = {
--     { app_id = "^Alacritty$" },
--     { title = "^scratch$" },             -- rules are OR'd across the list
--   },
--   excludes = { { app_id = "firefox$" } },
--   -- open-time options:
--   open_on_output = "eDP-1",
--   open_on_workspace = "browser",
--   open_fullscreen = false,
--   open_maximized = false,
--   open_floating = true,
--   default_column_width = { proportion = 0.5 },
--   default_window_height = { proportion = 0.5 },
--   default_column_display = "normal",     -- or "tabbed"
--   default_floating_position = { x = 100, y = 100, relative_to = "top_left" },
--   -- appearance overrides:
--   focus_ring = { active_color = "#ff0" },
--   border = { active_color = "#ff0" },
--   shadow = { draw_behind_window = false },
--   tab_indicator = { active_color = "#ff0" },
--   geometry_corner_radius = { top_left = 8, top_right = 8, bottom_left = 8, bottom_right = 8 },
--   clip_to_geometry = false,
--   draw_border_with_background = false,
--   -- behavior:
--   block_out_from = "screencast",         -- or "screen_capture"
--   min_width = 100, min_height = 100,
--   max_width = 2000, max_height = 2000,
--   opacity = 1.0,
--   variable_refresh_rate = true,
--   scroll_factor = { base = 1.0 },
-- }

--------------------------------------------------------------------------------
-- Layer rules (for layer-shell surfaces: bars, notifications, launchers)
--------------------------------------------------------------------------------

-- neori.layer_rule {
--   matches = { { namespace = "^notifications$" } },
--   -- block_out_from = "screencast",
--   -- opacity = 1.0,
--   -- geometry_corner_radius = { top_left = 12, top_right = 12 },
--   -- shadow = { on = true },
-- }

--------------------------------------------------------------------------------
-- Event handlers: startup, monitor-added/-removed, lid, tablet
--------------------------------------------------------------------------------
--
-- Any of these accept a string action ("spawn foo"), a table action
-- (neori.spawn("foo"), { close_window = true }, …), or a Lua function. Lua
-- functions get a connector-name string for the monitor events; see
-- https://neori-wm.github.io/neori/Lua-Scripting for the runtime API.

neori.on("startup", neori.spawn("waybar"))
-- neori.on("startup", neori.spawn_sh("sleep 1 && mako"))
-- neori.on("startup", function()
--   neori.action("create-virtual-output", { name = "sunshine" })
-- end)

-- neori.on("monitor-added", function(connector)
--   neori.action("spawn-sh", string.format("notify-send 'output connected' %q", connector))
-- end)
-- neori.on("monitor-removed", function(connector)
--   neori.action("spawn-sh", string.format("notify-send 'output removed' %q", connector))
-- end)

-- Switch events (also configurable as declarative actions via neori.switch_events).
-- neori.on("lid-open",       function() neori.action("spawn-sh", "notify-send 'lid open'") end)
-- neori.on("lid-closed",     function() neori.action("power-off-monitors") end)
-- neori.on("tablet-mode-on", function() neori.action("spawn-sh", "notify-send 'tablet mode on'")  end)
-- neori.on("tablet-mode-off",function() neori.action("spawn-sh", "notify-send 'tablet mode off'") end)

-- Declarative switch_events section (alternative to neori.on for lid/tablet).
-- neori.switch_events {
--   lid_closed = "spawn-sh notify-send lid-closed",
--   lid_open   = "spawn-sh notify-send lid-open",
--   tablet_mode_on  = "spawn-sh notify-send tablet-on",
--   tablet_mode_off = "spawn-sh notify-send tablet-off",
-- }

--------------------------------------------------------------------------------
-- Key bindings
--------------------------------------------------------------------------------
--
-- Keys consist of modifiers separated by + signs, followed by an XKB key
-- name in the end. "Mod" is a special modifier equal to Super when running
-- on a TTY, and to Alt when running as a winit window.
--
-- Most actions bound here can also be invoked programmatically with
-- `neori msg action do-something`.
--
-- The third form -- a Lua function -- lets you dispatch actions dynamically
-- via `neori.action(name, args)`. See the "Lua function callbacks" section at
-- the bottom of this file.

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

--------------------------------------------------------------------------------
-- Virtual output control (headless + Sunshine / Moonlight streaming)
--------------------------------------------------------------------------------
--
-- See Virtual-Outputs.md for Sunshine integration, headless workstation setup,
-- and the drain-windows-on-remove pattern.

-- neori.bind("Mod+Alt+V",       neori.create_virtual_output { name = "sunshine", width = 1920, height = 1080, refresh_rate = 144 })
-- neori.bind("Mod+Alt+Shift+V", neori.remove_virtual_output("sunshine"))

--------------------------------------------------------------------------------
-- Lua function callbacks and neori.action
--------------------------------------------------------------------------------
--
-- Any neori.bind / neori.on handler can be a Lua function. From inside that
-- function, call neori.action(name, args) to dispatch any IPC action; the
-- queued actions run in order right after the function returns.
--
-- See https://neori-wm.github.io/neori/Lua-Scripting for the full API and the
-- known limitations (no timers yet, function binds don't appear in the hotkey
-- overlay with their title).

-- A function bind that composes two actions:
-- neori.bind("Mod+Alt+G", function()
--   neori.action("focus_column_first")
--   neori.action("maximize_column")
-- end)

-- Function callback that decides at runtime what to do:
-- neori.bind("Mod+Alt+B", function()
--   local hour = tonumber(os.date("%H"))
--   if hour >= 20 or hour < 7 then
--     neori.action("spawn-sh", "brightnessctl set 20%")
--   else
--     neori.action("spawn-sh", "brightnessctl set 80%")
--   end
-- end)

-- Lua-held state, e.g. remembering whether a window was opened:
-- (For a scratchpad, prefer the real thing: see `neori.scratchpad` above.)
-- local scratch_spawned = false
-- neori.bind("Mod+Grave", function()
--   if not scratch_spawned then
--     neori.action("spawn", { "kitty", "--class", "neori-scratch" })
--     scratch_spawned = true
--   else
--     neori.action("toggle_window_floating")
--   end
-- end)

-- Window events + Window userdata.
--
-- Read-only methods: id(), app_id(), title(), is_focused(), is_floating(),
-- is_fullscreen(), workspace(), output().
--
-- Mutation methods: close(), focus(), fullscreen(true|false|nil),
-- float(true|false|nil), send_to_workspace(idx|name), send_to_output(name).
--
-- Pre-placement overrides (only valid inside window-opening handlers):
-- open_on_output(name), open_on_workspace(name), open_fullscreen(bool),
-- open_maximized(bool), open_floating(bool). These replace the matching
-- fields of the static `window_rule { ... }` resolution for that one window.

-- Pre-placement rule: Firefox opens on the "browser" workspace, fullscreen.
-- Runs before first paint, so there's no flash into a tiled state first.
-- neori.on("window-opening", function(w)
--   if w:app_id() == "firefox" then
--     w:open_on_workspace("browser")
--     w:open_fullscreen(true)
--   end
--   if (w:title() or ""):match("Picture%-in%-Picture") then
--     w:open_floating(true)
--   end
-- end)

-- Post-placement variant (window-opened) — good for toasts, focus swaps, or
-- anything that doesn't need to land before the first frame.
-- neori.on("window-opened", function(w)
--   if w:app_id() == "firefox" then
--     w:send_to_workspace("browser")
--   end
-- end)

-- Toast on focus change.
-- neori.on("window-focused", function(w)
--   neori.action("spawn-sh", string.format(
--     "notify-send 'focus' '%s'", w:app_id() or "?"))
-- end)

-- Close every Steam game window with one chord.
-- neori.bind("Mod+Alt+K", function()
--   for _, w in ipairs(neori.windows()) do
--     if (w:app_id() or ""):match("^steam_app_") then
--       w:close()
--     end
--   end
-- end)

-- Count floating windows and toast the number.
-- neori.bind("Mod+Alt+C", function()
--   local n = 0
--   for _, w in ipairs(neori.windows()) do
--     if w:is_floating() then n = n + 1 end
--   end
--   neori.action("spawn-sh", "notify-send 'floating:' '" .. n .. "'")
-- end)

-- Jump focus to the first terminal if there is one.
-- neori.bind("Mod+Alt+Return", function()
--   for _, w in ipairs(neori.windows()) do
--     local app = w:app_id() or ""
--     if app == "kitty" or app == "Alacritty" or app == "foot" then
--       w:focus()
--       return
--     end
--   end
-- end)

-- Toggle fullscreen on the focused window.
-- neori.bind("Mod+Alt+F11", function()
--   local w = neori.focused_window()
--   if w then w:fullscreen(not w:is_fullscreen()) end
-- end)

-- Float Picture-in-Picture players automatically when they open.
-- neori.on("window-opened", function(w)
--   if (w:title() or ""):match("Picture%-in%-Picture") then
--     w:float(true)
--   end
-- end)

-- Shove every Slack / Discord / Telegram window onto a side output.
-- neori.bind("Mod+Alt+S", function()
--   local side = "HDMI-A-1"   -- replace with your side-monitor connector
--   for _, w in ipairs(neori.windows()) do
--     local app = w:app_id() or ""
--     if app == "Slack" or app == "discord" or app == "org.telegram.desktop" then
--       w:send_to_output(side)
--     end
--   end
-- end)

--------------------------------------------------------------------------------
-- Output events + Output userdata.
--
-- Read-only methods: name(), make(), model(), serial(), mode(), modes(),
-- scale(), transform(), position(), is_focused(), is_virtual(), is_off(),
-- vrr_supported(), vrr_enabled().
--
-- Mutation methods: on(), off(), enable(bool|nil), set_mode("WxH@HZ"|"auto"),
-- set_scale(n|"auto"), set_transform("90"|...), set_position({x,y}|"auto"),
-- set_vrr(bool, on_demand?).
--
-- monitor-added / monitor-removed handlers now take a second argument:
--   neori.on("monitor-added", function(connector, out) ... end)
-- Old handlers that take only `connector` still work.

-- Set up a 4K monitor automatically on hotplug.
-- neori.on("monitor-added", function(connector, out)
--   if out:name() == "DP-3" then
--     out:set_mode("3840x2160@120")
--     out:set_scale(1.5)
--     out:set_vrr(true)
--   end
-- end)

-- Bump the focused output's scale a notch.
-- neori.bind("Mod+Alt+Plus", function()
--   local out = neori.focused_output()
--   if out then out:set_scale((out:scale() or 1) * 1.25) end
-- end)

-- Toggle fractional scaling on every real (non-virtual) output.
-- neori.bind("Mod+Alt+Minus", function()
--   for _, out in ipairs(neori.outputs()) do
--     if not out:is_virtual() then out:set_scale(1.0) end
--   end
-- end)

-- Enumerate outputs into a notification.
-- neori.bind("Mod+Alt+O", function()
--   local msg = {}
--   for _, out in ipairs(neori.outputs()) do
--     table.insert(msg, string.format("%s %s @%sx", out:name(), out:mode() or "?", out:scale() or "?"))
--   end
--   neori.action("spawn-sh", "notify-send 'outputs' '" .. table.concat(msg, "\\n") .. "'")
-- end)

--------------------------------------------------------------------------------
-- Workspace userdata.
--
-- Read-only methods: idx(), id(), name(), output(), is_focused(), windows().
-- Mutation methods:  focus(), send_to_output(name), set_name(s), unset_name().
--
-- `neori.workspaces()` returns an array of Workspace in global 1-based order,
-- `neori.focused_workspace()` returns the currently focused one (or nil).
-- ws:windows() returns the Window userdata on this workspace (same shape as
-- `neori.windows()`).

-- Rename the active workspace to the focused window's app_id.
-- neori.bind("Mod+Alt+R", function()
--   local ws = neori.focused_workspace()
--   local w  = neori.focused_window()
--   if ws and w and w:app_id() then ws:set_name(w:app_id()) end
-- end)

-- Clear the active workspace's name.
-- neori.bind("Mod+Shift+R", function()
--   local ws = neori.focused_workspace()
--   if ws then ws:unset_name() end
-- end)

-- Move the active workspace to the other monitor.
-- neori.bind("Mod+Alt+Left", function()
--   local ws = neori.focused_workspace()
--   for _, out in ipairs(neori.outputs()) do
--     if not out:is_focused() then ws:send_to_output(out:name()) return end
--   end
-- end)

-- Jump focus to the workspace named "code" (creating/renaming is manual: use
-- the entry pusher at config time, or `ws:set_name("code")` at runtime).
-- neori.bind("Mod+Alt+C", function()
--   for _, ws in ipairs(neori.workspaces()) do
--     if ws:name() == "code" then ws:focus() return end
--   end
-- end)

-- Workspace / output lifecycle events. Fire whenever the compositor notices
-- a change (per refresh tick). workspace-removed fires with the last-known
-- snapshot of the workspace that just vanished.
--
-- Note: `print(...)` output goes to $XDG_STATE_HOME/neori/lua.log (typically
-- ~/.local/state/neori/lua.log). Tail it with `tail -f`. For diagnostics, an
-- `error("...")` from a handler goes through neori's tracing log instead.
-- neori.on("workspace-focused", function(ws)
--   neori.action("spawn", { "notify-send", "ws", ws:name() or tostring(ws:idx()) })
-- end)
-- neori.on("workspace-added",   function(ws) print("ws +", ws:idx(), ws:name()) end)
-- neori.on("workspace-removed", function(ws) print("ws -", ws:idx(), ws:name()) end)
-- neori.on("output-focused",    function(out) print("output focus:", out:name()) end)

--------------------------------------------------------------------------------
-- Pointer position + cursor warp.
--
-- neori.pointer()      -> { x, y, output } in global logical coordinates, or nil
-- neori.warp(x, y)     queues a cursor warp (compositor drains after callback)

-- Jump the cursor to the top-left of the active output on demand.
-- neori.bind("Mod+Alt+Home", function()
--   local out = neori.focused_output()
--   if out and out:position() then
--     neori.warp(out:position().x + 10, out:position().y + 10)
--   end
-- end)

-- Toast the pointer's current output each time the active workspace changes.
-- neori.on("workspace-focused", function(ws)
--   local p = neori.pointer()
--   print("ws", ws:idx(), "pointer on", p and p.output or "?")
-- end)

--------------------------------------------------------------------------------
-- Keyboard layout.
--
-- neori.keyboard_layout() -> current layout name (e.g. "English (US)")
-- neori.on("layout-changed", function(name) ... end)

-- neori.on("layout-changed", function(name)
--   neori.action("spawn-sh", string.format("notify-send layout %q", name))
-- end)

--------------------------------------------------------------------------------
-- Input devices.
--
-- Read-only methods: name(), sysname(), product_id(), vendor_id(),
-- is_pointer(), is_keyboard(), is_touch(), is_touchpad().
--
-- Mutation methods: set_tap(bool), set_natural_scroll(bool), set_accel(speed),
-- set_left_handed(bool), set_disabled(bool). Mutations go directly to
-- libinput and persist until the next config reload wipes them.

-- Tune every touchpad at startup.
-- neori.on("startup", function()
--   for _, d in ipairs(neori.devices()) do
--     if d:is_touchpad() then
--       d:set_tap(true)
--       d:set_natural_scroll(true)
--       d:set_accel(0.3)
--     end
--   end
-- end)

-- Disable the built-in keyboard when any external keyboard is plugged in.
-- neori.bind("Mod+Alt+K", function()
--   local has_external = false
--   for _, d in ipairs(neori.devices()) do
--     if d:is_keyboard() and not d:name():match("Translated Set") then
--       has_external = true
--       break
--     end
--   end
--   for _, d in ipairs(neori.devices()) do
--     if d:is_keyboard() and d:name():match("Translated Set") then
--       d:set_disabled(has_external)
--     end
--   end
-- end)

-- Includes (load extra Lua files from the same directory):
-- neori.include("extra-binds.lua")
-- neori.include("plugins/*.lua")

-- Strict parsing (unknown keys become errors instead of warnings):
-- neori.strict(true)
