use std::str::FromStr;

use miette::miette;
use neori_config_derive::LuaTable;
use smithay::input::keyboard::{keysyms, Keysym, XkbConfig};
use smithay::reexports::input;

use crate::binds::Modifiers;
use crate::utils::{Flag, Percent};
use crate::FloatOrInt;

#[derive(Debug, Default, PartialEq, LuaTable)]
pub struct Input {
    pub keyboard: Keyboard,
    pub touchpad: Touchpad,
    pub mouse: Mouse,
    pub trackpoint: Trackpoint,
    pub trackball: Trackball,
    pub tablet: Tablet,
    pub touch: Touch,
    pub disable_power_key_handling: bool,
    #[lua(with = crate::lua_table::bool_or_table, lua_type = "boolean|table")]
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    #[lua(with = crate::lua_table::bool_or_table, lua_type = "boolean|table")]
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    pub workspace_auto_back_and_forth: bool,
    pub mod_key: Option<ModKey>,
    pub mod_key_nested: Option<ModKey>,
}

#[allow(dead_code)]
#[derive(Debug, Default, PartialEq)]
pub struct InputPart {
    pub keyboard: Option<KeyboardPart>,
    pub touchpad: Option<Touchpad>,
    pub mouse: Option<Mouse>,
    pub trackpoint: Option<Trackpoint>,
    pub trackball: Option<Trackball>,
    pub tablet: Option<Tablet>,
    pub touch: Option<Touch>,
    pub disable_power_key_handling: Option<Flag>,
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    pub workspace_auto_back_and_forth: Option<Flag>,
    pub mod_key: Option<ModKey>,
    pub mod_key_nested: Option<ModKey>,
}

#[derive(Debug, PartialEq, Eq, LuaTable)]
pub struct Keyboard {
    pub xkb: Xkb,
    pub repeat_delay: u16,
    pub repeat_rate: u8,
    pub track_layout: TrackLayout,
    pub numlock: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            xkb: Default::default(),
            repeat_delay: 600,
            repeat_rate: 25,
            track_layout: Default::default(),
            numlock: Default::default(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub struct KeyboardPart {
    pub xkb: Option<Xkb>,
    pub repeat_delay: Option<u16>,
    pub repeat_rate: Option<u8>,
    pub track_layout: Option<TrackLayout>,
    pub numlock: Option<Flag>,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, LuaTable)]
pub struct Xkb {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
    #[lua(with = crate::lua_table::xkb_file, lua_type = "string")]
    pub file: Option<XkbFile>,
}

impl Xkb {
    pub fn to_xkb_config(&self) -> XkbConfig<'_> {
        XkbConfig {
            rules: &self.rules,
            model: &self.model,
            layout: &self.layout,
            variant: &self.variant,
            options: self.options.clone(),
        }
    }
}

/// Contents of an xkb keymap file, read when parsing the config (`None` if reading failed).
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct XkbFile(pub Option<String>);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TrackLayout {
    #[default]
    Global,
    Window,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, LuaTable)]
pub struct ScrollFactor {
    pub base: Option<FloatOrInt<0, 100>>,
    pub horizontal: Option<FloatOrInt<-100, 100>>,
    pub vertical: Option<FloatOrInt<-100, 100>>,
}

impl ScrollFactor {
    pub fn h_v_factors(&self) -> (f64, f64) {
        let base_value = self.base.map(|f| f.0).unwrap_or(1.0);
        let h = self.horizontal.map(|f| f.0).unwrap_or(base_value);
        let v = self.vertical.map(|f| f.0).unwrap_or(base_value);
        (h, v)
    }
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Touchpad {
    pub off: bool,
    pub tap: bool,
    pub dwt: bool,
    pub dwtp: bool,
    pub drag: Option<bool>,
    pub drag_lock: bool,
    pub natural_scroll: bool,
    pub click_method: Option<ClickMethod>,
    pub accel_speed: FloatOrInt<-1, 1>,
    pub accel_profile: Option<AccelProfile>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub scroll_button_lock: bool,
    pub tap_button_map: Option<TapButtonMap>,
    pub left_handed: bool,
    pub disabled_on_external_mouse: bool,
    pub middle_emulation: bool,
    #[lua(with = crate::lua_table::scroll_factor, lua_type = "number|table")]
    pub scroll_factor: Option<ScrollFactor>,
    pub pinch_sensitivity: Option<FloatOrInt<0, 100>>,
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Mouse {
    pub off: bool,
    pub natural_scroll: bool,
    pub accel_speed: FloatOrInt<-1, 1>,
    pub accel_profile: Option<AccelProfile>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub scroll_button_lock: bool,
    pub left_handed: bool,
    pub middle_emulation: bool,
    #[lua(with = crate::lua_table::scroll_factor, lua_type = "number|table")]
    pub scroll_factor: Option<ScrollFactor>,
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Trackpoint {
    pub off: bool,
    pub natural_scroll: bool,
    pub accel_speed: FloatOrInt<-1, 1>,
    pub accel_profile: Option<AccelProfile>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub scroll_button_lock: bool,
    pub left_handed: bool,
    pub middle_emulation: bool,
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Trackball {
    pub off: bool,
    pub natural_scroll: bool,
    pub accel_speed: FloatOrInt<-1, 1>,
    pub accel_profile: Option<AccelProfile>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub scroll_button_lock: bool,
    pub left_handed: bool,
    pub middle_emulation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickMethod {
    Clickfinger,
    ButtonAreas,
}

impl From<ClickMethod> for input::ClickMethod {
    fn from(value: ClickMethod) -> Self {
        match value {
            ClickMethod::Clickfinger => Self::Clickfinger,
            ClickMethod::ButtonAreas => Self::ButtonAreas,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelProfile {
    Adaptive,
    Flat,
}

impl From<AccelProfile> for input::AccelProfile {
    fn from(value: AccelProfile) -> Self {
        match value {
            AccelProfile::Adaptive => Self::Adaptive,
            AccelProfile::Flat => Self::Flat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollMethod {
    NoScroll,
    TwoFinger,
    Edge,
    OnButtonDown,
}

impl From<ScrollMethod> for input::ScrollMethod {
    fn from(value: ScrollMethod) -> Self {
        match value {
            ScrollMethod::NoScroll => Self::NoScroll,
            ScrollMethod::TwoFinger => Self::TwoFinger,
            ScrollMethod::Edge => Self::Edge,
            ScrollMethod::OnButtonDown => Self::OnButtonDown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

impl From<TapButtonMap> for input::TapButtonMap {
    fn from(value: TapButtonMap) -> Self {
        match value {
            TapButtonMap::LeftRightMiddle => Self::LeftRightMiddle,
            TapButtonMap::LeftMiddleRight => Self::LeftMiddleRight,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Tablet {
    pub off: bool,
    pub calibration_matrix: Option<Vec<f32>>,
    pub map_to_output: Option<String>,
    pub map_to_focused_output: bool,
    pub map_to_focused_window: bool,
    pub left_handed: bool,
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Touch {
    pub off: bool,
    pub calibration_matrix: Option<Vec<f32>>,
    pub map_to_output: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, LuaTable)]
pub struct FocusFollowsMouse {
    pub outputs_only: Option<bool>,
    pub max_scroll_amount: Option<Percent>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, LuaTable)]
pub struct WarpMouseToFocus {
    pub mode: Option<WarpMouseToFocusMode>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WarpMouseToFocusMode {
    CenterXy,
    CenterXyAlways,
}

impl FromStr for WarpMouseToFocusMode {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "center-xy" => Ok(Self::CenterXy),
            "center-xy-always" => Ok(Self::CenterXyAlways),
            _ => Err(miette!(
                r#"invalid mode for warp-mouse-to-focus, can be "center-xy" or "center-xy-always" (or leave unset for separate centering)"#
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ModKey {
    Ctrl,
    Shift,
    Alt,
    Super,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModKey {
    pub fn to_modifiers(&self) -> Modifiers {
        match self {
            ModKey::Ctrl => Modifiers::CTRL,
            ModKey::Shift => Modifiers::SHIFT,
            ModKey::Alt => Modifiers::ALT,
            ModKey::Super => Modifiers::SUPER,
            ModKey::IsoLevel3Shift => Modifiers::ISO_LEVEL3_SHIFT,
            ModKey::IsoLevel5Shift => Modifiers::ISO_LEVEL5_SHIFT,
        }
    }

    /// Returns the modifier key for a keysym, if the keysym is a modifier key.
    pub fn from_keysym(keysym: Keysym) -> Option<Self> {
        match keysym.raw() {
            keysyms::KEY_Control_L | keysyms::KEY_Control_R => Some(Self::Ctrl),
            keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => Some(Self::Shift),
            keysyms::KEY_Alt_L | keysyms::KEY_Alt_R => Some(Self::Alt),
            keysyms::KEY_Super_L | keysyms::KEY_Super_R => Some(Self::Super),
            keysyms::KEY_ISO_Level3_Shift => Some(Self::IsoLevel3Shift),
            keysyms::KEY_ISO_Level5_Shift => Some(Self::IsoLevel5Shift),
            _ => None,
        }
    }

    pub fn matches_keysym(self, keysym: Keysym) -> bool {
        Self::from_keysym(keysym) == Some(self)
    }
}

impl FromStr for ModKey {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &*s.to_ascii_lowercase() {
            "ctrl" | "control" => Ok(Self::Ctrl),
            "shift" => Ok(Self::Shift),
            "alt" => Ok(Self::Alt),
            "super" | "win" => Ok(Self::Super),
            "iso_level3_shift" | "mod5" => Ok(Self::IsoLevel3Shift),
            "iso_level5_shift" | "mod3" => Ok(Self::IsoLevel5Shift),
            _ => Err(miette!("invalid Mod key: {s}")),
        }
    }
}

impl FromStr for ClickMethod {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "clickfinger" => Ok(Self::Clickfinger),
            "button-areas" => Ok(Self::ButtonAreas),
            _ => Err(miette!(
                r#"invalid click method, can be "button-areas" or "clickfinger""#
            )),
        }
    }
}

impl FromStr for AccelProfile {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "adaptive" => Ok(Self::Adaptive),
            "flat" => Ok(Self::Flat),
            _ => Err(miette!(
                r#"invalid accel profile, can be "adaptive" or "flat""#
            )),
        }
    }
}

impl FromStr for ScrollMethod {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "no-scroll" => Ok(Self::NoScroll),
            "two-finger" => Ok(Self::TwoFinger),
            "edge" => Ok(Self::Edge),
            "on-button-down" => Ok(Self::OnButtonDown),
            _ => Err(miette!(
                r#"invalid scroll method, can be "no-scroll", "two-finger", "edge", or "on-button-down""#
            )),
        }
    }
}

impl FromStr for TapButtonMap {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "left-right-middle" => Ok(Self::LeftRightMiddle),
            "left-middle-right" => Ok(Self::LeftMiddleRight),
            _ => Err(miette!(
                r#"invalid tap button map, can be "left-right-middle" or "left-middle-right""#
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use super::*;

    #[test]
    fn mod_key_names_are_consistent() {
        // The X11 `ModN` modifier indices are a separate numbering from the ISO level numbers,
        // and the two happen to be crossed over: ISO Level 3 Shift is Mod5, and ISO Level 5 Shift
        // is Mod3. This test guards against "fixing" this apparent mismatch.
        for (name, modifier, keysym, modifiers) in [
            (
                "Mod5",
                ModKey::IsoLevel3Shift,
                keysyms::KEY_ISO_Level3_Shift,
                Modifiers::ISO_LEVEL3_SHIFT,
            ),
            (
                "Mod3",
                ModKey::IsoLevel5Shift,
                keysyms::KEY_ISO_Level5_Shift,
                Modifiers::ISO_LEVEL5_SHIFT,
            ),
        ] {
            // Name -> variant.
            assert_eq!(
                name.parse::<ModKey>().unwrap(),
                modifier,
                "parsing `{name}`"
            );

            // Keysym -> variant.
            assert_eq!(
                ModKey::from_keysym(Keysym::from(keysym)),
                Some(modifier),
                "keysym for `{name}`",
            );
            assert!(modifier.matches_keysym(Keysym::from(keysym)));

            // Variant -> held modifiers.
            assert_eq!(modifier.to_modifiers(), modifiers, "modifiers for `{name}`");
        }

        // ISO_Level3_Shift and Mod5 are the same modifier, and so are ISO_Level5_Shift and Mod3.
        assert_eq!(
            "ISO_Level3_Shift".parse::<ModKey>().unwrap(),
            "Mod5".parse::<ModKey>().unwrap()
        );
        assert_eq!(
            "ISO_Level5_Shift".parse::<ModKey>().unwrap(),
            "Mod3".parse::<ModKey>().unwrap()
        );
    }

    #[test]
    fn scroll_factor_h_v_factors() {
        let sf = ScrollFactor {
            base: Some(FloatOrInt(2.0)),
            horizontal: None,
            vertical: None,
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r#"
        (
            2.0,
            2.0,
        )
        "#);

        let sf = ScrollFactor {
            base: None,
            horizontal: Some(FloatOrInt(3.0)),
            vertical: Some(FloatOrInt(-1.0)),
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r#"
        (
            3.0,
            -1.0,
        )
        "#);

        let sf = ScrollFactor {
            base: Some(FloatOrInt(2.0)),
            horizontal: Some(FloatOrInt(1.0)),
            vertical: None,
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r"
        (
            1.0,
            2.0,
        )
        ");
    }
}
