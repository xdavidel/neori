use crate::appearance::{Color, WorkspaceShadow, WorkspaceShadowPart, DEFAULT_BACKDROP_COLOR};
use crate::utils::Flag;
use crate::FloatOrInt;

#[derive(Debug, Clone, PartialEq)]
pub struct Cursor {
    pub xcursor_theme: String,
    pub xcursor_size: u8,
    pub hide_when_typing: bool,
    pub hide_after_inactive_ms: Option<u32>,
    pub shake_to_find: ShakeToFind,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            xcursor_theme: String::from("default"),
            xcursor_size: 24,
            hide_when_typing: false,
            hide_after_inactive_ms: None,
            shake_to_find: ShakeToFind::default(),
        }
    }
}

/// Enlarges the cursor when the user shakes the mouse, to help find it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeToFind {
    pub enable: bool,
    /// Multiplier applied to `xcursor_size` while the cursor is enlarged.
    pub size_multiplier: f64,
    /// Absolute xcursor size in px while enlarged. Overrides `size_multiplier` when set.
    pub size: Option<u8>,
    /// How long the cursor stays enlarged after the last detected shake.
    pub timeout_ms: u32,
    /// Duration of the grow/shrink animation.
    pub animation_ms: u32,
    /// Reversals older than this are forgotten.
    pub window_ms: u32,
    /// Minimum number of reversals within the window to count as a shake.
    pub min_reversals: u32,
    /// Deltas smaller than this (in pixels) are ignored as jitter.
    pub min_step: f64,
    /// Minimum direction change to count as a reversal, in degrees.
    pub reversal_angle_deg: f64,
}

impl Default for ShakeToFind {
    fn default() -> Self {
        Self {
            enable: false,
            size_multiplier: 4.0,
            size: None,
            timeout_ms: 1000,
            animation_ms: 250,
            window_ms: 600,
            min_reversals: 4,
            min_step: 12.0,
            reversal_angle_deg: 120.0,
        }
    }
}

impl ShakeToFind {
    /// Applies the fields set in `part`, leaving the rest unchanged.
    pub fn apply_part(&mut self, part: &ShakeToFindToSet) {
        if let Some(enable) = part.enable {
            self.enable = enable;
        }
        if let Some(v) = part.size_multiplier {
            self.size_multiplier = v;
        }
        if let Some(v) = part.size {
            self.size = Some(v);
        }
        if let Some(v) = part.timeout_ms {
            self.timeout_ms = v;
        }
        if let Some(v) = part.animation_ms {
            self.animation_ms = v;
        }
        if let Some(v) = part.window_ms {
            self.window_ms = v;
        }
        if let Some(v) = part.min_reversals {
            self.min_reversals = v;
        }
        if let Some(v) = part.min_step {
            self.min_step = v;
        }
        if let Some(v) = part.reversal_angle_deg {
            self.reversal_angle_deg = v;
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
    pub shake_to_find: Option<ShakeToFindToSet>,
}

pub use neori_ipc::ShakeToFindToSet;

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
