use crate::appearance::{Color, WorkspaceShadow, WorkspaceShadowPart, DEFAULT_BACKDROP_COLOR};
use crate::utils::Flag;
use crate::FloatOrInt;

#[derive(Debug, PartialEq)]
pub struct Cursor {
    pub xcursor_theme: String,
    pub xcursor_size: u8,
    pub hide_when_typing: bool,
    pub hide_after_inactive_ms: Option<u32>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            xcursor_theme: String::from("default"),
            xcursor_size: 24,
            hide_when_typing: false,
            hide_after_inactive_ms: None,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub struct CursorPart {
    pub xcursor_theme: Option<String>,
    pub xcursor_size: Option<u8>,
    pub hide_when_typing: Option<Flag>,
    pub hide_after_inactive_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotPath(pub Option<String>);

impl Default for ScreenshotPath {
    fn default() -> Self {
        Self(Some(String::from(
            "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png",
        )))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlay {
    pub skip_at_startup: bool,
    pub hide_not_bound: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlayPart {
    pub skip_at_startup: Option<Flag>,
    pub hide_not_bound: Option<Flag>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotification {
    pub disable_failed: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotificationPart {
    pub disable_failed: Option<Flag>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clipboard {
    pub disable_primary: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardPart {
    pub disable_primary: Option<Flag>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overview {
    pub zoom: f64,
    pub backdrop_color: Color,
    pub workspace_shadow: WorkspaceShadow,
}

impl Default for Overview {
    fn default() -> Self {
        Self {
            zoom: 0.5,
            backdrop_color: DEFAULT_BACKDROP_COLOR,
            workspace_shadow: WorkspaceShadow::default(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverviewPart {
    pub zoom: Option<FloatOrInt<0, 1>>,
    pub backdrop_color: Option<Color>,
    pub workspace_shadow: Option<WorkspaceShadowPart>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Environment(pub Vec<EnvironmentVariable>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellite {
    pub off: bool,
    pub path: String,
}

impl Default for XwaylandSatellite {
    fn default() -> Self {
        Self {
            off: false,
            path: String::from("xwayland-satellite"),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellitePart {
    pub off: bool,
    pub on: bool,
    pub path: Option<String>,
}
