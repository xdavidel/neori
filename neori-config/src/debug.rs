use std::path::PathBuf;

use crate::utils::Flag;

#[derive(Debug, Default, PartialEq)]
pub struct Debug {
    pub preview_render: Option<PreviewRender>,
    pub dbus_interfaces_in_non_session_instances: bool,
    pub wait_for_frame_completion_before_queueing: bool,
    pub enable_overlay_planes: bool,
    pub disable_cursor_plane: bool,
    pub disable_direct_scanout: bool,
    pub restrict_primary_scanout_to_matching_format: bool,
    pub force_disable_connectors_on_resume: bool,
    pub render_drm_device: Option<PathBuf>,
    pub ignored_drm_devices: Vec<PathBuf>,
    pub force_pipewire_invalid_modifier: bool,
    pub disable_pipewire_dmabuf: bool,
    pub emulate_zero_presentation_time: bool,
    pub disable_resize_throttling: bool,
    pub disable_transactions: bool,
    pub keep_laptop_panel_on_when_lid_is_closed: bool,
    pub disable_monitor_names: bool,
    pub strict_new_window_focus_policy: bool,
    pub honor_xdg_activation_with_invalid_serial: bool,
    pub deactivate_unfocused_windows: bool,
    pub skip_cursor_only_updates_during_vrr: bool,
    pub disable_10bit_output: bool,
    pub nested_show_host_cursor: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default, PartialEq)]
pub struct DebugPart {
    pub preview_render: Option<PreviewRender>,
    pub dbus_interfaces_in_non_session_instances: Option<Flag>,
    pub wait_for_frame_completion_before_queueing: Option<Flag>,
    pub enable_overlay_planes: Option<Flag>,
    pub disable_cursor_plane: Option<Flag>,
    pub disable_direct_scanout: Option<Flag>,
    pub restrict_primary_scanout_to_matching_format: Option<Flag>,
    pub force_disable_connectors_on_resume: Option<Flag>,
    pub render_drm_device: Option<PathBuf>,
    pub ignored_drm_devices: Vec<PathBuf>,
    pub force_pipewire_invalid_modifier: Option<Flag>,
    pub emulate_zero_presentation_time: Option<Flag>,
    pub disable_resize_throttling: Option<Flag>,
    pub disable_transactions: Option<Flag>,
    pub keep_laptop_panel_on_when_lid_is_closed: Option<Flag>,
    pub disable_monitor_names: Option<Flag>,
    pub strict_new_window_focus_policy: Option<Flag>,
    pub honor_xdg_activation_with_invalid_serial: Option<Flag>,
    pub deactivate_unfocused_windows: Option<Flag>,
    pub skip_cursor_only_updates_during_vrr: Option<Flag>,
    pub disable_10bit_output: Option<Flag>,
    pub disable_pipewire_dmabuf: Option<Flag>,
    pub nested_show_host_cursor: Option<Flag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewRender {
    Screencast,
    ScreenCapture,
}
