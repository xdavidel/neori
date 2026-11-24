//! Parsing config structs from Lua tables.
//!
//! Structs derive [`LuaTable`] (see the `neori-config-derive` crate), which turns each field into
//! a key parsed by its type's [`FromLuaConfig`] implementation. This module has those
//! implementations for the value types: numbers, strings, colors, regexes and so on.
//!
//! Parsing follows the config's strictness: an invalid value or an unknown key is a warning that
//! skips it, or an error in strict mode (`neori.strict(true)`).

use std::str::FromStr;

use mlua::Value;

use crate::appearance::{
    BlockOutFrom, Border, Color, CornerRadius, FocusRing, Gradient, GradientInterpolation,
    GradientRelativeTo, InsertHint, Shadow, TabIndicator, TabIndicatorLength, TabIndicatorPosition,
};
use crate::debug::PreviewRender;
use crate::input::{
    AccelProfile, ClickMethod, ModKey, ScrollFactor, ScrollMethod, TapButtonMap, TrackLayout,
    WarpMouseToFocusMode, XkbFile,
};
use crate::layout::{CenterFocusedColumn, DefaultPresetSize, PresetSize};
use crate::parse_lua::{
    invalid_config, key_str, parse_block_out_from, parse_css_color, unknown_key, value_is_truthy,
    value_to_corner_radius, value_to_default_preset_size, value_to_f64, value_to_i64,
    value_to_string,
};
use crate::utils::{Flag, Percent, RegexEq};
use crate::window_rule::{OnXdgActivate, RelativeTo, VrrRule};
use crate::FloatOrInt;

/// A value that can be read from Lua config.
pub trait FromLuaConfig: Sized {
    /// Parses `value`, found at `path` (like `window_rule.shadow.color`).
    ///
    /// `Ok(None)` means the value was invalid and was reported as a warning; the caller leaves
    /// the setting as it was. In strict mode that's an error instead.
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String>;

    /// The type, as LuaLS writes it, for the editor definitions.
    fn lua_type() -> String;

    /// Sets `self` from `value`. Replaces it, except for [`LuaTable`] structs, which only change
    /// the keys `value` has.
    fn merge_from_lua(&mut self, value: &Value, path: &str) -> Result<(), String> {
        if let Some(v) = Self::from_lua_config(value, path)? {
            *self = v;
        }
        Ok(())
    }
}

/// A config struct read from a Lua table key by key. Derived with `#[derive(LuaTable)]`.
pub trait LuaTable: Default {
    /// Sets the field for `key`. Returns `false` when the struct has no such key.
    fn set_lua_key(&mut self, key: &str, value: &Value, path: &str) -> Result<bool, String>;

    /// The keys, with their LuaLS types and doc comments.
    fn lua_fields() -> Vec<LuaField>;

    /// Sets every key in `table`, reporting the unknown ones.
    fn merge_lua_table(&mut self, table: &mlua::Table, path: &str) -> Result<(), String> {
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|e| format!("{path}: {e}"))?;
            let key = key_str(&key).map_err(|e| format!("{path}: {e}"))?;
            if !self.set_lua_key(&key, &value, path)? {
                unknown_key(path, &key)?;
            }
        }
        Ok(())
    }
}

/// A key of a [`LuaTable`] struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaField {
    pub name: &'static str,
    pub lua_type: String,
    pub doc: &'static str,
}

/// Parses a [`LuaTable`] struct from a table value, starting from its defaults.
pub fn table_from_lua<T: LuaTable>(value: &Value, path: &str) -> Result<Option<T>, String> {
    let Value::Table(table) = value else {
        invalid_config(&format!(
            "{path} must be a table, got {}",
            value.type_name()
        ))?;
        return Ok(None);
    };
    let mut this = T::default();
    this.merge_lua_table(table, path)?;
    Ok(Some(this))
}

/// Sets the keys of a table value on a [`LuaTable`] struct, keeping the others.
pub fn merge_table_from_lua<T: LuaTable>(
    this: &mut T,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let Value::Table(table) = value else {
        invalid_config(&format!(
            "{path} must be a table, got {}",
            value.type_name()
        ))?;
        return Ok(());
    };
    this.merge_lua_table(table, path)
}

/// Reports an invalid value at `path`, as a warning or a strict-mode error.
fn invalid<T>(path: &str, message: impl std::fmt::Display) -> Result<Option<T>, String> {
    invalid_config(&format!("{path}: {message}"))?;
    Ok(None)
}

/// Prefixes a conversion error with its path.
fn at<T>(path: &str, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|e| format!("{path}: {e}"))
}

impl FromLuaConfig for bool {
    fn from_lua_config(value: &Value, _path: &str) -> Result<Option<Self>, String> {
        Ok(Some(value_is_truthy(value)))
    }

    fn lua_type() -> String {
        "boolean".to_owned()
    }
}

impl FromLuaConfig for f64 {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        Ok(Some(at(path, value_to_f64(value))?))
    }

    fn lua_type() -> String {
        "number".to_owned()
    }
}

impl FromLuaConfig for f32 {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        Ok(Some(at(path, value_to_f64(value))? as f32))
    }

    fn lua_type() -> String {
        "number".to_owned()
    }
}

macro_rules! integer {
    ($($ty:ty),*) => {$(
        impl FromLuaConfig for $ty {
            fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
                let i = at(path, value_to_i64(value))?;
                match <$ty>::try_from(i) {
                    Ok(v) => Ok(Some(v)),
                    Err(_) => invalid(
                        path,
                        format!("must be between {} and {}, got {i}", <$ty>::MIN, <$ty>::MAX),
                    ),
                }
            }

            fn lua_type() -> String {
                "integer".to_owned()
            }
        }
    )*};
}
integer!(u8, u16, u32, i16, i32);

impl FromLuaConfig for String {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        Ok(Some(at(path, value_to_string(value))?))
    }

    fn lua_type() -> String {
        "string".to_owned()
    }
}

impl<const MIN: i32, const MAX: i32> FromLuaConfig for FloatOrInt<MIN, MAX> {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        let v = at(path, value_to_f64(value))?;
        if !(f64::from(MIN)..=f64::from(MAX)).contains(&v) {
            return invalid(path, format!("must be between {MIN} and {MAX}, got {v}"));
        }
        Ok(Some(FloatOrInt(v)))
    }

    fn lua_type() -> String {
        "number".to_owned()
    }
}

impl FromLuaConfig for Flag {
    fn from_lua_config(value: &Value, _path: &str) -> Result<Option<Self>, String> {
        Ok(Some(Flag(value_is_truthy(value))))
    }

    fn lua_type() -> String {
        "boolean".to_owned()
    }
}

/// One value or a list of them: `match = { app_id = "a" }` or `match = { { … }, { … } }`, and
/// `"DP-1"` or `{ "DP-1", "HDMI-A-1" }`.
impl<T: FromLuaConfig> FromLuaConfig for Vec<T> {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        if let Value::Table(table) = value {
            if table.raw_len() > 0 {
                let mut items = Vec::new();
                for (i, item) in table.clone().sequence_values::<Value>().enumerate() {
                    let item = at(path, item.map_err(|e| e.to_string()))?;
                    let item_path = format!("{path}[{}]", i + 1);
                    if let Some(item) = T::from_lua_config(&item, &item_path)? {
                        items.push(item);
                    }
                }
                return Ok(Some(items));
            }
        }
        Ok(T::from_lua_config(value, path)?.map(|item| vec![item]))
    }

    fn lua_type() -> String {
        let item = T::lua_type();
        format!("{item}|{item}[]")
    }
}

impl FromLuaConfig for Color {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        let s = at(path, value_to_string(value))?;
        match parse_css_color(&s) {
            Ok(color) => Ok(Some(color)),
            Err(e) => invalid(path, e),
        }
    }

    fn lua_type() -> String {
        "string".to_owned()
    }
}

/// `{ from = "#f00", to = "#00f", angle = 45, relative_to = "workspace-view", ["in"] = "oklch" }`.
/// Not derived: `from` and `to` have no defaults, and `in` is a Rust keyword.
impl FromLuaConfig for Gradient {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        let Value::Table(table) = value else {
            return invalid(path, format!("must be a table, got {}", value.type_name()));
        };
        let mut from = None;
        let mut to = None;
        let mut gradient = Gradient {
            from: Color::default(),
            to: Color::default(),
            angle: 180,
            relative_to: GradientRelativeTo::Window,
            in_: GradientInterpolation::default(),
        };
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = at(path, pair.map_err(|e| e.to_string()))?;
            let key = at(path, key_str(&key))?;
            let key_path = format!("{path}.{key}");
            match key.as_str() {
                "from" => from = Color::from_lua_config(&value, &key_path)?,
                "to" => to = Color::from_lua_config(&value, &key_path)?,
                "angle" => {
                    if let Some(angle) = i16::from_lua_config(&value, &key_path)? {
                        gradient.angle = angle;
                    }
                }
                "relative_to" | "relative-to" => {
                    if let Some(relative_to) =
                        GradientRelativeTo::from_lua_config(&value, &key_path)?
                    {
                        gradient.relative_to = relative_to;
                    }
                }
                "in" => {
                    let s = at(&key_path, value_to_string(&value))?;
                    match GradientInterpolation::from_str(&s) {
                        Ok(in_) => gradient.in_ = in_,
                        Err(e) => return invalid(&key_path, e),
                    }
                }
                _ => unknown_key(path, &key)?,
            }
        }
        let (Some(from), Some(to)) = (from, to) else {
            return invalid(path, "a gradient needs both `from` and `to` colors");
        };
        gradient.from = from;
        gradient.to = to;
        Ok(Some(gradient))
    }

    fn lua_type() -> String {
        "table".to_owned()
    }
}

/// Parses a string into one of `names`, reporting anything else.
fn one_of<T: Copy>(value: &Value, path: &str, names: &[(&str, T)]) -> Result<Option<T>, String> {
    let s = at(path, value_to_string(value))?;
    if let Some((_, v)) = names
        .iter()
        .find(|(name, _)| *name == s || name.replace('-', "_") == s)
    {
        return Ok(Some(*v));
    }
    let expected: Vec<String> = names
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    invalid(
        path,
        format!("invalid value `{s}`, expected {}", expected.join(", ")),
    )
}

/// The LuaLS type for a string that's one of `names`.
fn one_of_type<T>(names: &[(&str, T)]) -> String {
    let names: Vec<String> = names
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    names.join("|")
}

macro_rules! string_enum {
    ($ty:ty, $names:expr) => {
        impl FromLuaConfig for $ty {
            fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
                one_of(value, path, $names)
            }

            fn lua_type() -> String {
                one_of_type($names)
            }
        }
    };
}

string_enum!(
    GradientRelativeTo,
    &[
        ("window", GradientRelativeTo::Window),
        ("workspace-view", GradientRelativeTo::WorkspaceView),
    ]
);
string_enum!(
    RelativeTo,
    &[
        ("top-left", RelativeTo::TopLeft),
        ("top-right", RelativeTo::TopRight),
        ("bottom-left", RelativeTo::BottomLeft),
        ("bottom-right", RelativeTo::BottomRight),
        ("top", RelativeTo::Top),
        ("bottom", RelativeTo::Bottom),
        ("left", RelativeTo::Left),
        ("right", RelativeTo::Right),
    ]
);
string_enum!(
    OnXdgActivate,
    &[
        ("ignore", OnXdgActivate::Ignore),
        ("set-urgent", OnXdgActivate::SetUrgent),
        ("focus", OnXdgActivate::Focus),
    ]
);
string_enum!(
    neori_ipc::Layer,
    &[
        ("background", neori_ipc::Layer::Background),
        ("bottom", neori_ipc::Layer::Bottom),
        ("top", neori_ipc::Layer::Top),
        ("overlay", neori_ipc::Layer::Overlay),
    ]
);
string_enum!(
    neori_ipc::ColumnDisplay,
    &[
        ("normal", neori_ipc::ColumnDisplay::Normal),
        ("tabbed", neori_ipc::ColumnDisplay::Tabbed),
    ]
);

/// `true`, `false` or `"fullscreen-only"`.
impl FromLuaConfig for VrrRule {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        match value {
            Value::Boolean(true) => Ok(Some(VrrRule::On)),
            Value::Boolean(false) => Ok(Some(VrrRule::Off)),
            // Also accept bool strings to reduce confusion.
            _ => one_of(
                value,
                path,
                &[
                    ("true", VrrRule::On),
                    ("false", VrrRule::Off),
                    ("fullscreen-only", VrrRule::FullscreenOnly),
                ],
            ),
        }
    }

    fn lua_type() -> String {
        "boolean|\"fullscreen-only\"".to_owned()
    }
}

impl FromLuaConfig for RegexEq {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        let s = at(path, value_to_string(value))?;
        match regex::Regex::from_str(&s) {
            Ok(regex) => Ok(Some(RegexEq(regex))),
            Err(e) => invalid(path, format!("invalid regex `{s}`: {e}")),
        }
    }

    fn lua_type() -> String {
        "string".to_owned()
    }
}

/// A radius, or `{ top_left, top_right, bottom_right, bottom_left }`.
impl FromLuaConfig for CornerRadius {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        Ok(Some(at(path, value_to_corner_radius(value))?))
    }

    fn lua_type() -> String {
        "number|table".to_owned()
    }
}

/// `"screencast"`, or `{ "screencast", kind = "hide" }`.
impl FromLuaConfig for BlockOutFrom {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        at(path, parse_block_out_from(value))
    }

    fn lua_type() -> String {
        "string|table".to_owned()
    }
}

/// `{ proportion = 0.5 }`, `{ fixed = 800 }`, or `{}` to let the window choose.
impl FromLuaConfig for DefaultPresetSize {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        match value_to_default_preset_size(value) {
            Ok(size) => Ok(Some(size)),
            Err(e) => invalid(path, e),
        }
    }

    fn lua_type() -> String {
        "table".to_owned()
    }
}

/// `{ proportion = 0.5 }` or `{ fixed = 800 }`.
impl FromLuaConfig for PresetSize {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        match preset_size_or_none(value, path)? {
            Some(Some(size)) => Ok(Some(size)),
            Some(None) => invalid(path, "needs `proportion` or `fixed`"),
            None => Ok(None),
        }
    }

    fn lua_type() -> String {
        "table".to_owned()
    }
}

/// Like a [`PresetSize`], but `{}` means none.
pub fn preset_size_or_none(
    value: &Value,
    path: &str,
) -> Result<Option<Option<PresetSize>>, String> {
    let Value::Table(table) = value else {
        return invalid(path, format!("must be a table, got {}", value.type_name()));
    };
    let mut size = None;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = at(path, pair.map_err(|e| e.to_string()))?;
        let key = at(path, key_str(&key))?;
        let key_path = format!("{path}.{key}");
        match key.as_str() {
            "proportion" => {
                if let Some(v) = f64::from_lua_config(&value, &key_path)? {
                    size = Some(PresetSize::Proportion(v));
                }
            }
            "fixed" => {
                if let Some(v) = i32::from_lua_config(&value, &key_path)? {
                    size = Some(PresetSize::Fixed(v));
                }
            }
            _ => unknown_key(path, &key)?,
        }
    }
    Ok(Some(size))
}

/// `0.5`, or `{ total_proportion = 0.5 }`.
impl FromLuaConfig for TabIndicatorLength {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        let Value::Table(table) = value else {
            let total_proportion = f64::from_lua_config(value, path)?;
            return Ok(total_proportion.map(|v| TabIndicatorLength {
                total_proportion: Some(v),
            }));
        };
        let mut length = TabIndicatorLength {
            total_proportion: None,
        };
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = at(path, pair.map_err(|e| e.to_string()))?;
            let key = at(path, key_str(&key))?;
            match key.as_str() {
                "total_proportion" | "total-proportion" => {
                    let key_path = format!("{path}.total_proportion");
                    if let Some(v) = f64::from_lua_config(&value, &key_path)? {
                        length.total_proportion = Some(v);
                    }
                }
                _ => unknown_key(path, &key)?,
            }
        }
        Ok(Some(length))
    }

    fn lua_type() -> String {
        "number|table".to_owned()
    }
}

string_enum!(
    TabIndicatorPosition,
    &[
        ("left", TabIndicatorPosition::Left),
        ("right", TabIndicatorPosition::Right),
        ("top", TabIndicatorPosition::Top),
        ("bottom", TabIndicatorPosition::Bottom),
    ]
);
string_enum!(
    CenterFocusedColumn,
    &[
        ("never", CenterFocusedColumn::Never),
        ("always", CenterFocusedColumn::Always),
        ("on-overflow", CenterFocusedColumn::OnOverflow),
    ]
);

/// `#[lua(extra = …)]` for structs with an `off` field, so that `on = true` turns them on.
macro_rules! on_key {
    ($($name:ident: $ty:ty),*) => {$(
        pub fn $name(this: &mut $ty, key: &str, value: &Value, _path: &str) -> Result<bool, String> {
            if key != "on" {
                return Ok(false);
            }
            this.off = !value_is_truthy(value);
            Ok(true)
        }
    )*};
}
on_key!(
    focus_ring_on: FocusRing,
    border_on: Border,
    tab_indicator_on: TabIndicator,
    insert_hint_on: InsertHint
);

/// `#[lua(extra = …)]` for [`Shadow`], which has an `on` field: `off = true` turns it off.
pub fn shadow_off(
    this: &mut Shadow,
    key: &str,
    value: &Value,
    _path: &str,
) -> Result<bool, String> {
    if key != "off" {
        return Ok(false);
    }
    this.on = !value_is_truthy(value);
    Ok(true)
}

impl FromLuaConfig for std::path::PathBuf {
    fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
        Ok(Some(at(path, value_to_string(value))?.into()))
    }

    fn lua_type() -> String {
        "string".to_owned()
    }
}

string_enum!(
    PreviewRender,
    &[
        ("screencast", PreviewRender::Screencast),
        ("screen-capture", PreviewRender::ScreenCapture),
    ]
);

string_enum!(
    TrackLayout,
    &[
        ("global", TrackLayout::Global),
        ("window", TrackLayout::Window)
    ]
);
string_enum!(
    ClickMethod,
    &[
        ("clickfinger", ClickMethod::Clickfinger),
        ("button-areas", ClickMethod::ButtonAreas),
    ]
);
string_enum!(
    AccelProfile,
    &[
        ("adaptive", AccelProfile::Adaptive),
        ("flat", AccelProfile::Flat),
    ]
);
string_enum!(
    ScrollMethod,
    &[
        ("no-scroll", ScrollMethod::NoScroll),
        ("two-finger", ScrollMethod::TwoFinger),
        ("edge", ScrollMethod::Edge),
        ("on-button-down", ScrollMethod::OnButtonDown),
    ]
);
string_enum!(
    TapButtonMap,
    &[
        ("left-right-middle", TapButtonMap::LeftRightMiddle),
        ("left-middle-right", TapButtonMap::LeftMiddleRight),
    ]
);
string_enum!(
    WarpMouseToFocusMode,
    &[
        ("center-xy", WarpMouseToFocusMode::CenterXy),
        ("center-xy-always", WarpMouseToFocusMode::CenterXyAlways),
    ]
);

/// A value parsed with its `FromStr` implementation, for types whose names have aliases.
macro_rules! from_str_value {
    ($ty:ty, $lua_type:expr) => {
        impl FromLuaConfig for $ty {
            fn from_lua_config(value: &Value, path: &str) -> Result<Option<Self>, String> {
                let s = at(path, value_to_string(value))?;
                match <$ty>::from_str(&s) {
                    Ok(v) => Ok(Some(v)),
                    Err(e) => invalid(path, e),
                }
            }

            fn lua_type() -> String {
                $lua_type.to_owned()
            }
        }
    };
}
from_str_value!(
    ModKey,
    "\"ctrl\"|\"shift\"|\"alt\"|\"super\"|\"iso_level3_shift\"|\"iso_level5_shift\""
);
from_str_value!(Percent, "string");

/// For `#[lua(with = …)]` on a `scroll_factor`: a number sets the base factor, a table
/// `{ base, horizontal, vertical }` sets them separately.
pub fn scroll_factor(value: &Value, path: &str) -> Result<Option<Option<ScrollFactor>>, String> {
    if let Value::Table(_) = value {
        return Ok(table_from_lua::<ScrollFactor>(value, path)?.map(Some));
    }
    let base = FloatOrInt::<0, 100>::from_lua_config(value, path)?;
    Ok(base.map(|base| {
        Some(ScrollFactor {
            base: Some(base),
            horizontal: None,
            vertical: None,
        })
    }))
}

/// For `#[lua(with = …)]` on an optional feature: `true` turns it on with its defaults, a table
/// turns it on with these settings, and `false` turns it off.
pub fn bool_or_table<T: LuaTable>(value: &Value, path: &str) -> Result<Option<Option<T>>, String> {
    match value {
        Value::Table(_) => Ok(table_from_lua::<T>(value, path)?.map(Some)),
        Value::Boolean(true) => Ok(Some(Some(T::default()))),
        Value::Boolean(false) => Ok(Some(None)),
        _ => invalid(
            path,
            format!("must be true, false or a table, got {}", value.type_name()),
        ),
    }
}

/// For `#[lua(with = …)]` on `xkb.file`: a path, relative to the config file.
pub fn xkb_file(value: &Value, path: &str) -> Result<Option<Option<XkbFile>>, String> {
    let file = at(path, value_to_string(value))?;
    Ok(Some(Some(crate::parse_lua::read_xkb_file(&file))))
}

/// `#[lua(extra = …)]` for [`crate::RecentWindows`], which has an `on` field: `off = true` turns
/// it off.
pub fn recent_windows_off(
    this: &mut crate::RecentWindows,
    key: &str,
    value: &Value,
    _path: &str,
) -> Result<bool, String> {
    if key != "off" {
        return Ok(false);
    }
    this.on = !value_is_truthy(value);
    Ok(true)
}
