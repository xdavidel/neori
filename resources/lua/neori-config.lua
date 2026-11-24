---@meta
-- The config tables, generated from neori-config's config structs. Don't edit it: change
-- the structs, then regenerate it with
--   NEORI_UPDATE_LUA_DEFINITIONS=1 cargo test -p neori-config --test lua-definitions

---Environment variables for programs neori spawns: name = value, `false` unsets one.
---@class neori.config.Environment
---@overload fun(config: table<string, string|false>)
---@field [string] string|false

---Binds: key = action, like `["Mod+T"] = "spawn foot"`.
---@alias neori.config.Binds table<string, string|table|function>

---Keybinding modes: name = binds.
---@alias neori.config.Modes table<string, neori.config.Binds>

---@class neori.config.Input
---@overload fun(config: neori.config.Input)
---@field keyboard? neori.config.Keyboard
---@field touchpad? neori.config.Touchpad
---@field mouse? neori.config.Mouse
---@field trackpoint? neori.config.Trackpoint
---@field trackball? neori.config.Trackball
---@field tablet? neori.config.Tablet
---@field touch? neori.config.Touch
---@field disable_power_key_handling? boolean
---@field warp_mouse_to_focus? boolean|table
---@field focus_follows_mouse? boolean|table
---@field workspace_auto_back_and_forth? boolean
---@field mod_key? "ctrl"|"shift"|"alt"|"super"|"iso_level3_shift"|"iso_level5_shift"
---@field mod_key_nested? "ctrl"|"shift"|"alt"|"super"|"iso_level3_shift"|"iso_level5_shift"

---@class neori.config.Animations
---@overload fun(config: neori.config.Animations)
---@field off? boolean
---@field slowdown? number
---@field workspace_switch? boolean|table
---@field window_open? boolean|table
---@field window_close? boolean|table
---@field horizontal_view_movement? boolean|table
---@field window_movement? boolean|table
---@field window_resize? boolean|table
---@field config_notification_open_close? boolean|table
---@field exit_confirmation_open_close? boolean|table
---@field screenshot_ui_open? boolean|table
---@field overview_open_close? boolean|table
---@field recent_windows_close? boolean|table

---@class neori.config.SwitchBinds
---@overload fun(config: neori.config.SwitchBinds)
---@field lid_open? neori.config.SwitchAction
---@field lid_close? neori.config.SwitchAction
---@field tablet_mode_on? neori.config.SwitchAction
---@field tablet_mode_off? neori.config.SwitchAction

---@class neori.config.Layout
---@overload fun(config: neori.config.Layout)
---@field focus_ring? neori.config.FocusRing
---@field border? neori.config.Border
---@field shadow? neori.config.Shadow
---@field tab_indicator? neori.config.TabIndicator
---@field insert_hint? neori.config.InsertHint
---@field preset_column_widths? table|table[]
---@field default_column_width? table `{ proportion = 0.5 }` or `{ fixed = 800 }`; `{}` lets windows choose.
---@field preset_window_heights? table|table[]
---@field center_focused_column? "never"|"always"|"on-overflow"
---@field always_center_single_column? boolean
---@field empty_workspace_above_first? boolean
---@field default_column_display? "normal"|"tabbed"
---@field gaps? number
---@field struts? neori.config.Struts
---@field background_color? string
---@field fullscreen_backdrop_color? string

---@class neori.config.Cursor
---@overload fun(config: neori.config.Cursor)
---@field xcursor_theme? string
---@field xcursor_size? integer
---@field hide_when_typing? boolean
---@field hide_after_inactive_ms? integer
---@field shake_to_find? boolean|table

---@class neori.config.Gestures
---@overload fun(config: neori.config.Gestures)
---@field dnd_edge_view_scroll? neori.config.DndEdgeViewScroll
---@field dnd_edge_workspace_switch? neori.config.DndEdgeWorkspaceSwitch
---@field hot_corners? neori.config.HotCorners

---@class neori.config.RecentWindows
---@overload fun(config: neori.config.RecentWindows)
---@field on? boolean
---@field debounce_ms? integer
---@field open_delay_ms? integer
---@field highlight? neori.config.MruHighlight
---@field previews? neori.config.MruPreviews
---@field binds? table

---@class neori.config.HotkeyOverlay
---@overload fun(config: neori.config.HotkeyOverlay)
---@field skip_at_startup? boolean
---@field hide_not_bound? boolean

---@class neori.config.Overview
---@overload fun(config: neori.config.Overview)
---@field zoom? number
---@field backdrop_color? string
---@field workspace_shadow? neori.config.WorkspaceShadow

---@class neori.config.Blur
---@overload fun(config: neori.config.Blur)
---@field off? boolean
---@field passes? integer
---@field offset? number
---@field noise? number
---@field saturation? number

---@class neori.config.Clipboard
---@overload fun(config: neori.config.Clipboard)
---@field disable_primary? boolean

---@class neori.config.ConfigNotification
---@overload fun(config: neori.config.ConfigNotification)
---@field disable_failed? boolean

---@class neori.config.XwaylandSatellite
---@overload fun(config: neori.config.XwaylandSatellite)
---@field off? boolean
---@field path? string

---@class neori.config.Debug
---@overload fun(config: neori.config.Debug)
---@field preview_render? "screencast"|"screen-capture"
---@field dbus_interfaces_in_non_session_instances? boolean
---@field wait_for_frame_completion_before_queueing? boolean
---@field enable_overlay_planes? boolean
---@field disable_cursor_plane? boolean
---@field disable_direct_scanout? boolean
---@field restrict_primary_scanout_to_matching_format? boolean
---@field force_disable_connectors_on_resume? boolean
---@field render_drm_device? string
---@field ignore_drm_device? string|string[]
---@field force_pipewire_invalid_modifier? boolean
---@field disable_pipewire_dmabuf? boolean
---@field emulate_zero_presentation_time? boolean
---@field disable_resize_throttling? boolean
---@field disable_transactions? boolean
---@field keep_laptop_panel_on_when_lid_is_closed? boolean
---@field disable_monitor_names? boolean
---@field strict_new_window_focus_policy? boolean
---@field honor_xdg_activation_with_invalid_serial? boolean
---@field deactivate_unfocused_windows? boolean
---@field skip_cursor_only_updates_during_vrr? boolean
---@field disable_10bit_output? boolean
---@field nested_show_host_cursor? boolean

---@class neori.config.WindowRule
---@field matches? neori.config.Match|neori.config.Match[]
---@field excludes? neori.config.Match|neori.config.Match[]
---@field default_column_width? table
---@field default_window_height? table
---@field open_on_output? string|string[] Outputs to open the window on, in preference order.
---@field open_on_workspace? string
---@field open_maximized? boolean
---@field open_maximized_to_edges? boolean
---@field open_fullscreen? boolean
---@field open_floating? boolean
---@field open_focused? boolean
---@field on_xdg_activate? "ignore"|"set-urgent"|"focus"
---@field min_width? integer
---@field min_height? integer
---@field max_width? integer
---@field max_height? integer
---@field focus_ring? neori.config.BorderRule
---@field border? neori.config.BorderRule
---@field shadow? neori.config.ShadowRule
---@field tab_indicator? neori.config.TabIndicatorRule
---@field draw_border_with_background? boolean
---@field opacity? number
---@field geometry_corner_radius? number|table
---@field clip_to_geometry? boolean
---@field baba_is_float? boolean
---@field block_out_from? string|table
---@field variable_refresh_rate? boolean|"fullscreen-only"
---@field default_column_display? "normal"|"tabbed"
---@field default_floating_position? neori.config.FloatingPosition
---@field scroll_factor? number
---@field pinch_sensitivity? number
---@field tiled_state? boolean
---@field background_effect? neori.config.BackgroundEffectRule
---@field popups? neori.config.PopupsRule

---@class neori.config.LayerRule
---@field matches? neori.config.LayerMatch|neori.config.LayerMatch[]
---@field excludes? neori.config.LayerMatch|neori.config.LayerMatch[]
---@field opacity? number
---@field block_out_from? string|table
---@field shadow? neori.config.ShadowRule
---@field geometry_corner_radius? number|table
---@field place_within_backdrop? boolean
---@field focus_on_type_in_overview? boolean
---@field baba_is_float? boolean
---@field background_effect? neori.config.BackgroundEffectRule
---@field popups? neori.config.PopupsRule

---@class neori.config.Output
---@field off? boolean
---@field create_virtual? boolean
---@field name? string
---@field scale? number
---@field transform? "normal"|"90"|"180"|"270"|"flipped"|"flipped-90"|"flipped-180"|"flipped-270"
---@field position? neori.config.Position
---@field max_bpc? integer|string
---@field mode? string
---@field modeline? table
---@field variable_refresh_rate? boolean|table
---@field drm_lease? boolean
---@field focus_at_startup? boolean
---@field background_color? string
---@field backdrop_color? string
---@field hot_corners? neori.config.HotCorners
---@field layout? neori.config.LayoutPart

---@class neori.config.Workspace
---@field name? string
---@field open_on_output? string|string[] Output names to open this workspace on, in preference order.
---@field layout? table

---@class neori.config.Scratchpad
---@field name? string Scratchpad name; empty string for the default scratchpad.
---@field spawn? string|string[] Command (argv) to spawn when toggling this scratchpad while empty.
---@field spawn_sh? string Shell command to spawn when toggling this scratchpad while empty.

---@class neori.config.Keyboard
---@field xkb? neori.config.Xkb
---@field repeat_delay? integer
---@field repeat_rate? integer
---@field track_layout? "global"|"window"
---@field numlock? boolean

---@class neori.config.Touchpad
---@field off? boolean
---@field tap? boolean
---@field dwt? boolean
---@field dwtp? boolean
---@field drag? boolean
---@field drag_lock? boolean
---@field natural_scroll? boolean
---@field click_method? "clickfinger"|"button-areas"
---@field accel_speed? number
---@field accel_profile? "adaptive"|"flat"
---@field scroll_method? "no-scroll"|"two-finger"|"edge"|"on-button-down"
---@field scroll_button? integer
---@field scroll_button_lock? boolean
---@field tap_button_map? "left-right-middle"|"left-middle-right"
---@field left_handed? boolean
---@field disabled_on_external_mouse? boolean
---@field middle_emulation? boolean
---@field scroll_factor? number|table
---@field pinch_sensitivity? number

---@class neori.config.Mouse
---@field off? boolean
---@field natural_scroll? boolean
---@field accel_speed? number
---@field accel_profile? "adaptive"|"flat"
---@field scroll_method? "no-scroll"|"two-finger"|"edge"|"on-button-down"
---@field scroll_button? integer
---@field scroll_button_lock? boolean
---@field left_handed? boolean
---@field middle_emulation? boolean
---@field scroll_factor? number|table

---@class neori.config.Trackpoint
---@field off? boolean
---@field natural_scroll? boolean
---@field accel_speed? number
---@field accel_profile? "adaptive"|"flat"
---@field scroll_method? "no-scroll"|"two-finger"|"edge"|"on-button-down"
---@field scroll_button? integer
---@field scroll_button_lock? boolean
---@field left_handed? boolean
---@field middle_emulation? boolean

---@class neori.config.Trackball
---@field off? boolean
---@field natural_scroll? boolean
---@field accel_speed? number
---@field accel_profile? "adaptive"|"flat"
---@field scroll_method? "no-scroll"|"two-finger"|"edge"|"on-button-down"
---@field scroll_button? integer
---@field scroll_button_lock? boolean
---@field left_handed? boolean
---@field middle_emulation? boolean

---@class neori.config.Tablet
---@field off? boolean
---@field calibration_matrix? number|number[]
---@field map_to_output? string
---@field map_to_focused_output? boolean
---@field map_to_focused_window? boolean
---@field left_handed? boolean

---@class neori.config.Touch
---@field off? boolean
---@field calibration_matrix? number|number[]
---@field map_to_output? string

---@class neori.config.SwitchAction
---@field spawn? string|string[]

---@class neori.config.FocusRing
---@field off? boolean
---@field width? number
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.Border
---@field off? boolean
---@field width? number
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.Shadow
---@field on? boolean
---@field offset? neori.config.ShadowOffset
---@field softness? number
---@field spread? number
---@field draw_behind_window? boolean
---@field color? string
---@field inactive_color? string

---@class neori.config.TabIndicator
---@field off? boolean
---@field hide_when_single_tab? boolean
---@field place_within_column? boolean
---@field gap? number
---@field width? number
---@field length? number|table
---@field position? "left"|"right"|"top"|"bottom"
---@field gaps_between_tabs? number
---@field corner_radius? number
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.InsertHint
---@field off? boolean
---@field color? string
---@field gradient? table

---@class neori.config.Struts
---@field left? number
---@field right? number
---@field top? number
---@field bottom? number

---@class neori.config.DndEdgeViewScroll
---@field trigger_width? number
---@field delay_ms? integer
---@field max_speed? number

---@class neori.config.DndEdgeWorkspaceSwitch
---@field trigger_height? number
---@field delay_ms? integer
---@field max_speed? number

---@class neori.config.HotCorners
---@field off? boolean
---@field top_left? boolean
---@field top_right? boolean
---@field bottom_left? boolean
---@field bottom_right? boolean

---@class neori.config.MruHighlight
---@field active_color? string
---@field urgent_color? string
---@field padding? number
---@field corner_radius? number

---@class neori.config.MruPreviews
---@field max_height? number
---@field max_scale? number

---@class neori.config.WorkspaceShadow
---@field off? boolean
---@field offset? neori.config.ShadowOffset
---@field softness? number
---@field spread? number
---@field color? string

---@class neori.config.Match
---@field app_id? string
---@field title? string
---@field is_active? boolean
---@field is_focused? boolean
---@field is_active_in_column? boolean
---@field is_floating? boolean
---@field is_window_cast_target? boolean
---@field is_urgent? boolean
---@field at_startup? boolean

---@class neori.config.BorderRule
---@field off? boolean
---@field on? boolean
---@field width? number
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.ShadowRule
---@field off? boolean
---@field on? boolean
---@field offset? neori.config.ShadowOffset
---@field softness? number
---@field spread? number
---@field draw_behind_window? boolean
---@field color? string
---@field inactive_color? string

---@class neori.config.TabIndicatorRule
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.FloatingPosition
---@field x? number
---@field y? number
---@field relative_to? "top-left"|"top-right"|"bottom-left"|"bottom-right"|"top"|"bottom"|"left"|"right"

---@class neori.config.BackgroundEffectRule
---@field xray? boolean
---@field blur? boolean
---@field noise? number
---@field saturation? number

---@class neori.config.PopupsRule
---@field opacity? number
---@field geometry_corner_radius? number|table
---@field background_effect? neori.config.BackgroundEffectRule

---@class neori.config.LayerMatch
---@field namespace? string
---@field at_startup? boolean
---@field layer? "background"|"bottom"|"top"|"overlay"

---@class neori.config.Position
---@field x? integer
---@field y? integer

---@class neori.config.LayoutPart
---@field focus_ring? neori.config.BorderRule
---@field border? neori.config.BorderRule
---@field shadow? neori.config.ShadowRule
---@field tab_indicator? neori.config.TabIndicatorPart
---@field insert_hint? neori.config.InsertHintPart
---@field preset_column_widths? table|table[]
---@field default_column_width? table
---@field preset_window_heights? table|table[]
---@field center_focused_column? "never"|"always"|"on-overflow"
---@field always_center_single_column? boolean
---@field empty_workspace_above_first? boolean
---@field default_column_display? "normal"|"tabbed"
---@field gaps? number
---@field struts? neori.config.Struts
---@field background_color? string
---@field fullscreen_backdrop_color? string

---@class neori.config.Xkb
---@field rules? string
---@field model? string
---@field layout? string
---@field variant? string
---@field options? string
---@field file? string

---@class neori.config.ShadowOffset
---@field x? number
---@field y? number

---@class neori.config.TabIndicatorPart
---@field off? boolean
---@field on? boolean
---@field hide_when_single_tab? boolean
---@field place_within_column? boolean
---@field gap? number
---@field width? number
---@field length? number|table
---@field position? "left"|"right"|"top"|"bottom"
---@field gaps_between_tabs? number
---@field corner_radius? number
---@field active_color? string
---@field inactive_color? string
---@field urgent_color? string
---@field active_gradient? table
---@field inactive_gradient? table
---@field urgent_gradient? table

---@class neori.config.InsertHintPart
---@field off? boolean
---@field on? boolean
---@field color? string
---@field gradient? table
