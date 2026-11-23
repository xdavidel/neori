//! neori config parsing.
//!
//! The config can be constructed from multiple files (includes). To support this, many types are
//! split into two. For example, `Layout` and `LayoutPart` where `Layout` is the final config and
//! `LayoutPart` is one part parsed from one config file.
//!
//! The convention for `Default` impls is to set the initial values before the parsing occurs.
//! Then, parsing will update the values with those parsed from the config.
//!
//! The `Default` values match those from `default-config.lua` in almost all cases, with a notable
//! exception of `binds {}` and some window rules.

#[macro_use]
extern crate tracing;

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use miette::{miette, Context as _, IntoDiagnostic as _};

#[macro_use]
pub mod macros;

pub mod animations;
pub mod appearance;
pub mod binds;
pub mod context;
pub mod debug;
pub mod error;
pub mod gestures;
pub mod input;
pub mod keycode;
pub mod layer_rule;
pub mod layout;
pub mod misc;
pub mod output;
pub mod parse_lua;
pub mod recent_windows;
pub mod scratchpad;
pub mod signals;
pub mod utils;
pub mod window_rule;
pub mod workspace;

/// Re-exported so downstream crates (compositor, tests) can read Lua values via
/// [`ScriptHost::read_global`] without taking a direct dependency on `mlua`.
pub use mlua;

pub use crate::animations::{Animation, Animations};
pub use crate::appearance::*;
pub use crate::binds::*;
pub use crate::context::{with_script_context, ScriptContext};
pub use crate::debug::Debug;
pub use crate::error::{ConfigIncludeError, ConfigParseResult};
pub use crate::gestures::Gestures;
pub use crate::input::{Input, ModKey, ScrollMethod, TrackLayout, WarpMouseToFocusMode, Xkb};
pub use crate::keycode::*;
pub use crate::layer_rule::LayerRule;
pub use crate::layout::*;
pub use crate::misc::*;
pub use crate::output::{Output, OutputName, Outputs, Position, Vrr};
pub use crate::parse_lua::{
    while_reloading, DeviceOp, DeviceSnapshot, LuaDevice, LuaOutput, LuaTimer, LuaWorkspace,
    OpenOverrides, OutputSnapshot, PointerSnapshot, PrintSink, PropertyValue, ScriptHost, Window,
    WindowGeometry, WindowSnapshot, WorkspaceSnapshot,
};
pub use crate::recent_windows::{MruDirection, MruFilter, MruPreviews, MruScope, RecentWindows};
pub use crate::scratchpad::Scratchpad;
pub use crate::signals::SignalTarget;
pub use crate::utils::FloatOrInt;
pub use crate::window_rule::{
    FloatingPosition, OnXdgActivate, PopupsRule, RelativeTo, ResolvedPopupsRules, WindowRule,
};
pub use crate::workspace::{Workspace, WorkspaceLayoutPart};

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub input: Input,
    pub outputs: Outputs,
    pub startup: Vec<Action>,
    pub monitor_added: Vec<Action>,
    pub monitor_removed: Vec<Action>,
    pub lid_open: Vec<Action>,
    pub lid_closed: Vec<Action>,
    pub tablet_mode_on: Vec<Action>,
    pub tablet_mode_off: Vec<Action>,
    pub layout: Layout,
    pub prefer_no_csd: bool,
    pub cursor: Cursor,
    pub screenshot_path: ScreenshotPath,
    pub clipboard: Clipboard,
    pub hotkey_overlay: HotkeyOverlay,
    pub config_notification: ConfigNotification,
    pub animations: Animations,
    pub blur: Blur,
    pub gestures: Gestures,
    pub overview: Overview,
    pub environment: Environment,
    pub xwayland_satellite: XwaylandSatellite,
    pub window_rules: Vec<WindowRule>,
    pub layer_rules: Vec<LayerRule>,
    pub binds: Binds,
    pub switch_events: SwitchBinds,
    pub debug: Debug,
    pub workspaces: Vec<Workspace>,
    pub scratchpads: Vec<Scratchpad>,
    /// Keybinding modes, declared with `neori.mode`, sorted by name.
    pub modes: Vec<BindMode>,
    pub recent_windows: RecentWindows,
}

#[derive(Debug, Clone)]
pub enum ConfigPath {
    /// Explicitly set config path.
    ///
    /// Load the config only from this path, never create it.
    Explicit(PathBuf),

    /// Default config path.
    ///
    /// Prioritize the user path, fallback to the system path, fallback to creating the user path
    /// at compositor startup.
    Regular {
        /// User config path, usually `$XDG_CONFIG_HOME/neori/config.lua`.
        user_path: PathBuf,
        /// System config path, usually `/etc/neori/config.lua`.
        system_path: PathBuf,
    },
}

impl Config {
    pub fn load_default() -> Self {
        let res = Config::parse(
            Path::new("default-config.lua"),
            include_str!("../../resources/default-config.lua"),
        );

        res.config.unwrap()
    }

    pub fn load(path: &Path) -> ConfigParseResult<Self, miette::Report> {
        let contents = match fs::read_to_string(path) {
            Ok(x) => x,
            Err(err) => {
                return ConfigParseResult::from_err(
                    miette!(err).context(format!("error reading {path:?}")),
                );
            }
        };

        Self::parse(path, &contents).map_config_res(|res| {
            let config = res.context("error parsing")?;
            debug!("loaded config from {path:?}");
            Ok(config)
        })
    }

    pub fn parse(path: &Path, text: &str) -> ConfigParseResult<Self, ConfigIncludeError> {
        match parse_lua::parse_lua(path, text) {
            Ok(parsed) => ConfigParseResult {
                config: Ok(parsed.config),
                includes: std::iter::once(path.to_path_buf())
                    .chain(parsed.includes)
                    .collect(),
            },
            Err(err) => ConfigParseResult {
                config: Err(ConfigIncludeError {
                    main: err.message,
                    includes: err
                        .includes
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                }),
                includes: err.includes,
            },
        }
    }

    pub fn parse_mem(text: &str) -> Result<Self, ConfigIncludeError> {
        Self::parse(Path::new("config.lua"), text).config
    }

    /// Like [`Self::parse`] but returns the [`ScriptHost`] with the config, so the caller
    /// can keep firing events at registered Lua callbacks.
    pub fn parse_with_host(
        path: &Path,
        text: &str,
    ) -> ConfigParseResult<(Config, ScriptHost), ConfigIncludeError> {
        match parse_lua::parse_lua_with_host(path, text) {
            Ok(parsed) => ConfigParseResult {
                config: Ok((parsed.config, parsed.host)),
                includes: std::iter::once(path.to_path_buf())
                    .chain(parsed.includes)
                    .collect(),
            },
            Err(err) => ConfigParseResult {
                config: Err(ConfigIncludeError {
                    main: err.message,
                    includes: err
                        .includes
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                }),
                includes: err.includes,
            },
        }
    }

    pub fn load_with_host(path: &Path) -> ConfigParseResult<(Config, ScriptHost), miette::Report> {
        let contents = match fs::read_to_string(path) {
            Ok(x) => x,
            Err(err) => {
                return ConfigParseResult::from_err(
                    miette!(err).context(format!("error reading {path:?}")),
                );
            }
        };

        Self::parse_with_host(path, &contents).map_config_res(|res| {
            let (config, host) = res.context("error parsing")?;
            debug!("loaded config from {path:?}");
            Ok((config, host))
        })
    }
}

impl ConfigPath {
    /// Loads the config, returns an error if it doesn't exist.
    pub fn load(&self) -> ConfigParseResult<Config, miette::Report> {
        let _span = tracy_client::span!("ConfigPath::load");

        self.load_inner(|user_path, system_path| {
            Err(miette!(
                "no config file found; create one at {user_path:?} or {system_path:?}",
            ))
        })
        .map_config_res(|res| res.context("error loading config"))
    }

    /// Like [`Self::load`] but also returns the [`ScriptHost`] for the loaded config.
    pub fn load_with_host(&self) -> ConfigParseResult<(Config, ScriptHost), miette::Report> {
        let _span = tracy_client::span!("ConfigPath::load");

        self.load_inner_with_host(|user_path, system_path| {
            Err(miette!(
                "no config file found; create one at {user_path:?} or {system_path:?}",
            ))
        })
        .map_config_res(|res| res.context("error loading config"))
    }

    /// Loads the config, or creates it if it doesn't exist.
    ///
    /// Returns a tuple containing the path that was created, if any, and the loaded config.
    ///
    /// If the config was created, but for some reason could not be read afterwards,
    /// this may return `(Some(_), Err(_))`.
    pub fn load_or_create(&self) -> (Option<&Path>, ConfigParseResult<Config, miette::Report>) {
        let _span = tracy_client::span!("ConfigPath::load_or_create");

        let mut created_at = None;

        let result = self
            .load_inner(|user_path, _| {
                Self::create(user_path, &mut created_at)
                    .map(|()| user_path)
                    .with_context(|| format!("error creating config at {user_path:?}"))
            })
            .map_config_res(|res| res.context("error loading config"));

        (created_at, result)
    }

    /// Like [`Self::load_or_create`] but also returns the [`ScriptHost`] for the loaded config.
    pub fn load_or_create_with_host(
        &self,
    ) -> (
        Option<&Path>,
        ConfigParseResult<(Config, ScriptHost), miette::Report>,
    ) {
        let _span = tracy_client::span!("ConfigPath::load_or_create");

        let mut created_at = None;

        let result = self
            .load_inner_with_host(|user_path, _| {
                Self::create(user_path, &mut created_at)
                    .map(|()| user_path)
                    .with_context(|| format!("error creating config at {user_path:?}"))
            })
            .map_config_res(|res| res.context("error loading config"));

        (created_at, result)
    }

    fn load_inner<'a>(
        &'a self,
        maybe_create: impl FnOnce(&'a Path, &'a Path) -> miette::Result<&'a Path>,
    ) -> ConfigParseResult<Config, miette::Report> {
        let path = match self {
            ConfigPath::Explicit(path) => path.as_path(),
            ConfigPath::Regular {
                user_path,
                system_path,
            } => {
                if user_path.exists() {
                    user_path.as_path()
                } else if system_path.exists() {
                    system_path.as_path()
                } else {
                    match maybe_create(user_path.as_path(), system_path.as_path()) {
                        Ok(x) => x,
                        Err(err) => return ConfigParseResult::from_err(miette!(err)),
                    }
                }
            }
        };
        Config::load(path)
    }

    fn load_inner_with_host<'a>(
        &'a self,
        maybe_create: impl FnOnce(&'a Path, &'a Path) -> miette::Result<&'a Path>,
    ) -> ConfigParseResult<(Config, ScriptHost), miette::Report> {
        let path = match self {
            ConfigPath::Explicit(path) => path.as_path(),
            ConfigPath::Regular {
                user_path,
                system_path,
            } => {
                if user_path.exists() {
                    user_path.as_path()
                } else if system_path.exists() {
                    system_path.as_path()
                } else {
                    match maybe_create(user_path.as_path(), system_path.as_path()) {
                        Ok(x) => x,
                        Err(err) => return ConfigParseResult::from_err(miette!(err)),
                    }
                }
            }
        };
        Config::load_with_host(path)
    }

    fn create<'a>(path: &'a Path, created_at: &mut Option<&'a Path>) -> miette::Result<()> {
        if let Some(default_parent) = path.parent() {
            fs::create_dir_all(default_parent)
                .into_diagnostic()
                .with_context(|| format!("error creating config directory {default_parent:?}"))?;
        }

        // Create the config and fill it with the default config if it doesn't exist.
        let mut new_file = match File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            res => res,
        }
        .into_diagnostic()
        .with_context(|| format!("error opening config file at {path:?}"))?;

        *created_at = Some(path);

        let default = include_bytes!("../../resources/default-config.lua");

        new_file
            .write_all(default)
            .into_diagnostic()
            .with_context(|| format!("error writing default config to {path:?}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use smithay::input::keyboard::Keysym;

    use super::*;
    use crate::output::{MaxBpc, Mode};

    /// A [`ScriptContext`] serving fixed compositor state, for running Lua callbacks in tests.
    #[derive(Default)]
    struct FakeContext {
        windows: Vec<WindowSnapshot>,
        focused: Option<u64>,
        outputs: Vec<OutputSnapshot>,
        focused_output: Option<String>,
        workspaces: Vec<WorkspaceSnapshot>,
        focused_workspace: Option<u64>,
        pointer: Option<PointerSnapshot>,
        devices: Vec<DeviceSnapshot>,
        keyboard_layout: Option<String>,
        /// Changes requested from Lua, in order.
        actions: Vec<Action>,
        output_ops: Vec<(String, neori_ipc::OutputAction)>,
        warps: Vec<(f64, f64)>,
        device_ops: Vec<(String, DeviceOp)>,
        /// Timer arm and disarm calls: `(id, Some(timeout ms))` or `(id, None)`.
        timer_calls: Vec<(u64, Option<u128>)>,
    }

    impl FakeContext {
        fn new(windows: Vec<WindowSnapshot>, focused: Option<u64>) -> Self {
            Self {
                windows,
                focused,
                ..Default::default()
            }
        }

        fn with_outputs(outputs: Vec<OutputSnapshot>, focused: Option<&str>) -> Self {
            Self {
                outputs,
                focused_output: focused.map(String::from),
                ..Default::default()
            }
        }

        fn with_workspaces(workspaces: Vec<WorkspaceSnapshot>, focused: Option<u64>) -> Self {
            Self {
                workspaces,
                focused_workspace: focused,
                ..Default::default()
            }
        }

        fn with_devices(devices: Vec<DeviceSnapshot>) -> Self {
            Self {
                devices,
                ..Default::default()
            }
        }
    }

    impl ScriptContext for FakeContext {
        fn windows(&mut self) -> Vec<WindowSnapshot> {
            self.windows.clone()
        }

        fn focused_window(&mut self) -> Option<WindowSnapshot> {
            let id = self.focused?;
            self.window(id)
        }

        fn outputs(&mut self) -> Vec<OutputSnapshot> {
            self.outputs.clone()
        }

        fn focused_output(&mut self) -> Option<OutputSnapshot> {
            let name = self.focused_output.clone()?;
            self.output(&name)
        }

        fn workspaces(&mut self) -> Vec<WorkspaceSnapshot> {
            self.workspaces.clone()
        }

        fn focused_workspace(&mut self) -> Option<WorkspaceSnapshot> {
            let id = self.focused_workspace?;
            self.workspace(id)
        }

        fn pointer(&mut self) -> Option<PointerSnapshot> {
            self.pointer.clone()
        }

        fn devices(&mut self) -> Vec<DeviceSnapshot> {
            self.devices.clone()
        }

        fn keyboard_layout(&mut self) -> Option<String> {
            self.keyboard_layout.clone()
        }

        fn do_action(&mut self, action: Action) {
            self.actions.push(action);
        }

        fn output_action(&mut self, output: &str, action: neori_ipc::OutputAction) {
            self.output_ops.push((output.to_owned(), action));
        }

        fn warp(&mut self, x: f64, y: f64) {
            self.warps.push((x, y));
        }

        fn device_op(&mut self, sysname: &str, op: DeviceOp) {
            self.device_ops.push((sysname.to_owned(), op));
        }

        fn arm_timer(&mut self, id: u64, timeout: std::time::Duration) {
            self.timer_calls.push((id, Some(timeout.as_millis())));
        }

        fn disarm_timer(&mut self, id: u64) {
            self.timer_calls.push((id, None));
        }
    }

    /// Runs bind 1 with `ctx` as the context, and returns the actions it carried out.
    fn run_bind(host: &ScriptHost, ctx: &mut FakeContext) -> Vec<Action> {
        with_script_context(ctx, || host.call_bind_function(1)).unwrap();
        std::mem::take(&mut ctx.actions)
    }

    #[test]
    fn can_create_default_config() {
        let _ = Config::load_default();
    }

    #[test]
    fn default_config_file_parses() {
        let text = include_str!("../../resources/default-config.lua");
        let result = Config::parse(std::path::Path::new("default-config.lua"), text);
        let config = match result.config {
            Ok(config) => config,
            Err(err) => panic!("default config failed to parse:\n{err}"),
        };
        assert!(!actions(&config).is_empty(), "default config has no binds");
    }

    #[test]
    fn default_repeat_params() {
        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.input.keyboard.repeat_delay, 600);
        assert_eq!(config.input.keyboard.repeat_rate, 25);
    }

    #[test]
    fn shake_to_find_is_disabled_by_default() {
        let config = Config::parse_mem("").unwrap();
        assert!(!config.cursor.shake_to_find.enable);
    }

    #[test]
    fn shake_to_find_bool_enables() {
        let config = Config::parse_mem("neori.cursor { shake_to_find = true }").unwrap();
        let shake = &config.cursor.shake_to_find;
        assert!(shake.enable);
        assert_eq!(shake.size_multiplier, 4.);
        assert_eq!(shake.size, None);
    }

    #[test]
    fn shake_to_find_table_parses() {
        let config = Config::parse_mem(
            r#"
            neori.cursor {
              xcursor_size = 32,
              shake_to_find = {
                size_multiplier = 3,
                size = 128,
                timeout_ms = 1500,
                animation_ms = 300,
                window_ms = 800,
                min_reversals = 3,
                min_step = 6.5,
                reversal_angle_deg = 135,
              },
            }
            "#,
        )
        .unwrap();
        let shake = &config.cursor.shake_to_find;
        assert!(shake.enable);
        assert_eq!(shake.size_multiplier, 3.);
        assert_eq!(shake.size, Some(128));
        assert_eq!(shake.timeout_ms, 1500);
        assert_eq!(shake.animation_ms, 300);
        assert_eq!(shake.window_ms, 800);
        assert_eq!(shake.min_reversals, 3);
        assert_eq!(shake.min_step, 6.5);
        assert_eq!(shake.reversal_angle_deg, 135.);
    }

    #[test]
    fn set_shake_to_find_ipc_action_applies_partial_update() {
        let action = neori_ipc::Action::SetShakeToFind {
            settings: neori_ipc::ShakeToFindToSet {
                enable: Some(true),
                size_multiplier: Some(3.),
                size: None,
                timeout_ms: Some(1500),
                animation_ms: None,
                window_ms: Some(800),
                min_reversals: Some(2),
                min_step: None,
                reversal_angle_deg: Some(135.),
            },
        };

        let Action::SetShakeToFind { settings: part } = action else {
            panic!("unexpected action");
        };

        let mut shake = ShakeToFind::default();
        shake.apply_part(&part);

        assert!(shake.enable);
        assert_eq!(shake.size_multiplier, 3.);
        assert_eq!(shake.size, None);
        assert_eq!(shake.timeout_ms, 1500);
        assert_eq!(shake.animation_ms, 250);
        assert_eq!(shake.window_ms, 800);
        assert_eq!(shake.min_reversals, 2);
        assert_eq!(shake.min_step, 12.);
        assert_eq!(shake.reversal_angle_deg, 135.);
    }

    #[test]
    fn shake_to_find_rejects_invalid_sensitivity() {
        for (key, value) in [
            ("window_ms", "0"),
            ("min_reversals", "0"),
            ("min_step", "-1"),
            ("reversal_angle_deg", "180"),
        ] {
            let err = Config::parse_mem(&format!(
                "neori.cursor {{ shake_to_find = {{ {key} = {value} }} }}"
            ))
            .unwrap_err();
            assert!(err.main.contains(key), "unexpected error for {key}: {err}");
        }
    }

    #[test]
    fn shake_to_find_rejects_non_positive_multiplier() {
        let err = Config::parse_mem("neori.cursor { shake_to_find = { size_multiplier = 0 } }")
            .unwrap_err();
        assert!(
            err.main.contains("size_multiplier"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn enlarge_cursor_action_parses() {
        let config = Config::parse_mem(r#"neori.bind("Mod+Shift+F", "enlarge-cursor")"#).unwrap();
        assert!(actions(&config).contains(&Action::EnlargeCursor {}));
    }

    #[test]
    fn output_api_registers_lookup_functions() {
        // neori.outputs() and neori.focused_output() should be callable from any Lua
        // context. They return empty / nil respectively when the cache is unseeded.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("startup", function()
                outputs_count = #neori.outputs()
                focused_name = neori.focused_output() and neori.focused_output():name() or nil
            end)
            "#,
        )
        .config
        .unwrap();

        host.call_event("startup").unwrap();

        let count = host.read_global("outputs_count").unwrap();
        assert!(matches!(count, mlua::Value::Integer(0)));
        let focused = host.read_global("focused_name").unwrap();
        assert!(matches!(focused, mlua::Value::Nil));
    }

    #[test]
    fn output_userdata_methods_return_snapshot_fields() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            captured = {}
            neori.on("startup", function()
                local out = neori.focused_output()
                captured.name = out:name()
                captured.make = out:make()
                captured.model = out:model()
                captured.mode = out:mode()
                captured.scale = out:scale()
                captured.transform = out:transform()
                captured.focused = out:is_focused()
                captured.virt = out:is_virtual()
                captured.vrr_supported = out:vrr_supported()
                captured.vrr_enabled = out:vrr_enabled()
                captured.pos_x = out:position() and out:position().x or nil
                captured.pos_y = out:position() and out:position().y or nil
                captured.modes_count = #out:modes()
            end)
            "#,
        )
        .config
        .unwrap();

        let snap = OutputSnapshot {
            name: "HEADLESS-1".to_string(),
            make: Some("neori".to_string()),
            model: Some("HEADLESS-1".to_string()),
            serial: None,
            mode: Some("1920x1080@60.000".to_string()),
            modes: vec![
                "1920x1080@60.000".to_string(),
                "1280x720@60.000".to_string(),
            ],
            scale: Some(1.25),
            transform: Some("90".to_string()),
            position: Some((100, 200)),
            is_focused: true,
            is_virtual: true,
            off: false,
            vrr_supported: true,
            vrr_enabled: false,
        };
        let name = snap.name.clone();
        let mut ctx = FakeContext::with_outputs(vec![snap], Some(&name));
        with_script_context(&mut ctx, || host.call_event("startup")).unwrap();

        let captured = host.read_global("captured").unwrap();
        let mlua::Value::Table(t) = captured else {
            panic!("captured is not a table");
        };
        assert_eq!(t.get::<String>("name").unwrap(), "HEADLESS-1");
        assert_eq!(t.get::<String>("make").unwrap(), "neori");
        assert_eq!(t.get::<String>("model").unwrap(), "HEADLESS-1");
        assert_eq!(t.get::<String>("mode").unwrap(), "1920x1080@60.000");
        assert_eq!(t.get::<f64>("scale").unwrap(), 1.25);
        assert_eq!(t.get::<String>("transform").unwrap(), "90");
        assert!(t.get::<bool>("focused").unwrap());
        assert!(t.get::<bool>("virt").unwrap());
        assert!(t.get::<bool>("vrr_supported").unwrap());
        assert!(!t.get::<bool>("vrr_enabled").unwrap());
        assert_eq!(t.get::<i64>("pos_x").unwrap(), 100);
        assert_eq!(t.get::<i64>("pos_y").unwrap(), 200);
        assert_eq!(t.get::<i64>("modes_count").unwrap(), 2);
    }

    #[test]
    fn output_mutation_methods_apply_ops() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("startup", function()
                local out = neori.focused_output()
                out:off()
                out:on()
                out:enable(true)    -- snapshot has off=false already, should be no-op
                out:enable(false)   -- toggles to off
                out:set_mode("2560x1440@144")
                out:set_mode("auto")
                out:set_scale(2.0)
                out:set_scale("auto")
                out:set_transform("90")
                out:set_position({ x = 100, y = 200 })
                out:set_position("auto")
                out:set_vrr(true)
            end)
            "#,
        )
        .config
        .unwrap();

        let snap = OutputSnapshot {
            name: "eDP-1".to_string(),
            off: false,
            ..Default::default()
        };
        let name = snap.name.clone();
        let mut ctx = FakeContext::with_outputs(vec![snap], Some(&name));
        with_script_context(&mut ctx, || host.call_event("startup")).unwrap();

        let ops = std::mem::take(&mut ctx.output_ops);
        let kinds: Vec<&str> = ops
            .iter()
            .map(|(_, op)| match op {
                neori_ipc::OutputAction::Off => "off",
                neori_ipc::OutputAction::On => "on",
                neori_ipc::OutputAction::Mode { .. } => "mode",
                neori_ipc::OutputAction::Scale { .. } => "scale",
                neori_ipc::OutputAction::Transform { .. } => "transform",
                neori_ipc::OutputAction::Position { .. } => "position",
                neori_ipc::OutputAction::Vrr { .. } => "vrr",
                _ => "other",
            })
            .collect();
        // Expected: off, on, (enable(true) no-op skipped), off (from enable(false)),
        // mode, mode (auto), scale, scale (auto), transform, position, position (auto), vrr.
        assert_eq!(
            kinds,
            vec![
                "off",
                "on",
                "off",
                "mode",
                "mode",
                "scale",
                "scale",
                "transform",
                "position",
                "position",
                "vrr",
            ],
            "unexpected ops: {kinds:?}"
        );

        // Each op targeted eDP-1.
        for (name, _) in &ops {
            assert_eq!(name, "eDP-1");
        }

        // Spot-check arg shapes.
        for (_, op) in &ops {
            if let neori_ipc::OutputAction::Mode { mode } = op {
                match mode {
                    neori_ipc::ModeToSet::Automatic => {}
                    neori_ipc::ModeToSet::Specific(m) => {
                        assert_eq!(m.width, 2560);
                        assert_eq!(m.height, 1440);
                        assert_eq!(m.refresh, Some(144.0));
                    }
                }
            }
            if let neori_ipc::OutputAction::Scale { scale } = op {
                match scale {
                    neori_ipc::ScaleToSet::Automatic => {}
                    neori_ipc::ScaleToSet::Specific(s) => assert_eq!(*s, 2.0),
                }
            }
            if let neori_ipc::OutputAction::Transform { transform } = op {
                assert_eq!(*transform, neori_ipc::Transform::_90);
            }
            if let neori_ipc::OutputAction::Vrr { vrr } = op {
                assert!(vrr.vrr);
                assert!(!vrr.on_demand);
            }
        }
    }

    #[test]
    fn workspace_api_registers_lookup_functions() {
        // neori.workspaces() and neori.focused_workspace() should be callable from any
        // Lua context. Empty / nil when the cache is unseeded.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("startup", function()
                workspaces_count = #neori.workspaces()
                focused_name = neori.focused_workspace() and neori.focused_workspace():name() or nil
            end)
            "#,
        )
        .config
        .unwrap();

        host.call_event("startup").unwrap();

        let count = host.read_global("workspaces_count").unwrap();
        assert!(matches!(count, mlua::Value::Integer(0)));
        let focused = host.read_global("focused_name").unwrap();
        assert!(matches!(focused, mlua::Value::Nil));
    }

    #[test]
    fn workspace_userdata_methods_return_snapshot_fields() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            captured = {}
            neori.on("startup", function()
                local ws = neori.focused_workspace()
                captured.idx = ws:idx()
                captured.id = ws:id()
                captured.name = ws:name()
                captured.output = ws:output()
                captured.focused = ws:is_focused()
                captured.window_count = #ws:windows()
                captured.first_window_id = ws:windows()[1] and ws:windows()[1]:id() or nil
            end)
            "#,
        )
        .config
        .unwrap();

        let ws_snap = WorkspaceSnapshot {
            idx: 3,
            name: Some("browser".to_string()),
            output_name: Some("eDP-1".to_string()),
            is_focused: true,
            id: 1234,
            window_ids: vec![42, 7],
        };
        // ws:windows() filters the live windows by window_ids.
        let focused_ws = Some(ws_snap.id);
        let mut ctx = FakeContext::new(
            vec![
                WindowSnapshot {
                    id: 42,
                    app_id: Some("firefox".into()),
                    ..Default::default()
                },
                WindowSnapshot {
                    id: 99,
                    app_id: Some("vim".into()),
                    ..Default::default()
                },
                WindowSnapshot {
                    id: 7,
                    app_id: Some("gimp".into()),
                    ..Default::default()
                },
            ],
            None,
        );
        ctx.workspaces = vec![ws_snap];
        ctx.focused_workspace = focused_ws;
        with_script_context(&mut ctx, || host.call_event("startup")).unwrap();

        let captured = host.read_global("captured").unwrap();
        let mlua::Value::Table(t) = captured else {
            panic!("captured is not a table");
        };
        assert_eq!(t.get::<u8>("idx").unwrap(), 3);
        assert_eq!(t.get::<u64>("id").unwrap(), 1234);
        assert_eq!(t.get::<String>("name").unwrap(), "browser");
        assert_eq!(t.get::<String>("output").unwrap(), "eDP-1");
        assert!(t.get::<bool>("focused").unwrap());
        // 42 and 7 are on the workspace; 99 is not.
        assert_eq!(t.get::<i64>("window_count").unwrap(), 2);
        // Either order is acceptable — just assert we see one of them.
        let fw: u64 = t.get("first_window_id").unwrap();
        assert!(fw == 42 || fw == 7, "unexpected first window id {fw}");
    }

    #[test]
    fn multiple_handlers_on_same_event_all_fire() {
        // neori.on(ev, fn) registers; registering twice should queue both
        // callbacks and both should run (in registration order).
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = {}
            neori.on("startup", function() seen[#seen + 1] = "a" end)
            neori.on("startup", function() seen[#seen + 1] = "b" end)
            "#,
        )
        .config
        .unwrap();

        host.call_event("startup").unwrap();

        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        assert_eq!(t.get::<String>(1).unwrap(), "a");
        assert_eq!(t.get::<String>(2).unwrap(), "b");
    }

    #[test]
    fn handler_error_does_not_skip_sibling_handlers() {
        // If the first handler throws, the second must still run. Current
        // call_event_with short-circuits on the first error — this test
        // documents the fix (handler isolation).
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            after_error_ran = false
            neori.on("startup", function() error("boom") end)
            neori.on("startup", function() after_error_ran = true end)
            "#,
        )
        .config
        .unwrap();

        // The call itself returns Err (an error occurred), but the sibling
        // handler must have had a chance to run.
        let _ = host.call_event("startup");
        let v = host.read_global("after_error_ran").unwrap();
        assert!(
            matches!(v, mlua::Value::Boolean(true)),
            "sibling handler must still run after earlier handler errored, got {v:?}"
        );
    }

    #[test]
    fn actions_happen_even_if_a_later_handler_fails() {
        // Actions are carried out as soon as neori.action is called, so a sibling handler that
        // fails afterwards doesn't undo them.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("startup", function() neori.action("close-window") end)
            neori.on("startup", function() error("boom") end)
            "#,
        )
        .config
        .unwrap();

        let mut ctx = FakeContext::default();
        let err = with_script_context(&mut ctx, || host.call_event("startup")).unwrap_err();
        assert!(err.contains("boom"), "{err}");
        assert_eq!(ctx.actions, [Action::CloseWindow { id: None }]);
    }

    #[test]
    fn print_routes_to_installed_sink() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        // Shared buffer the test reads back; a tiny Write adapter forwards
        // bytes into it so we can wrap it as a PrintSink.
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let sink: PrintSink = Arc::new(Mutex::new(
            Box::new(Capture(Arc::clone(&captured))) as Box<dyn Write + Send>
        ));

        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("startup", function()
                print("hello", 42, true, nil)
            end)
            "#,
        )
        .config
        .unwrap();

        host.set_print_log(sink);
        host.call_event("startup").unwrap();

        let out = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            out.contains("hello\t42\ttrue\tnil"),
            "expected tab-joined args in output, got: {out:?}"
        );
        assert!(out.ends_with('\n'), "line must end with newline: {out:?}");
        assert!(
            out.starts_with('['),
            "line must start with a timestamp bracket: {out:?}"
        );
    }

    #[test]
    fn devices_api_returns_devices_and_methods_apply_ops() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = {}
            neori.bind("Mod+Alt+D", function()
                seen.count = #neori.devices()
                local names, syses = {}, {}
                for i, d in ipairs(neori.devices()) do
                    names[i] = d:name()
                    syses[i] = d:sysname()
                    if d:is_touchpad() then
                        d:set_tap(true)
                        d:set_natural_scroll(true)
                        d:set_accel(0.5)
                    end
                    if d:is_pointer() and not d:is_touchpad() then
                        d:set_left_handed(true)
                    end
                    if d:is_keyboard() then
                        d:set_disabled(true)
                    end
                end
                seen.names = names
                seen.syses = syses
            end)
            "#,
        )
        .config
        .unwrap();

        // No devices: devices() returns an empty array.
        let mut ctx = FakeContext::default();
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert!(ctx.device_ops.is_empty());
        let count_cold: i64 = {
            let seen = host.read_global("seen").unwrap();
            let mlua::Value::Table(t) = seen else {
                panic!("seen is not a table");
            };
            t.get("count").unwrap()
        };
        assert_eq!(count_cold, 0);

        // Seed with three devices: a touchpad, a mouse, a keyboard.
        let mut ctx = FakeContext::with_devices(vec![
            DeviceSnapshot {
                name: "SynPS/2 Touchpad".into(),
                sysname: "event4".into(),
                is_pointer: true,
                is_touchpad: true,
                ..Default::default()
            },
            DeviceSnapshot {
                name: "Logitech MX Master".into(),
                sysname: "event6".into(),
                is_pointer: true,
                ..Default::default()
            },
            DeviceSnapshot {
                name: "AT Translated Set 2 keyboard".into(),
                sysname: "event3".into(),
                is_keyboard: true,
                ..Default::default()
            },
        ]);
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        let ops = std::mem::take(&mut ctx.device_ops);
        assert_eq!(
            ops.len(),
            5,
            "expected 5 device ops (tap+natural+accel for pad, left-handed for mouse, disabled for kb), got {ops:?}"
        );

        let by_sys = |target: &str| -> Vec<&DeviceOp> {
            ops.iter()
                .filter(|(s, _)| s == target)
                .map(|(_, o)| o)
                .collect()
        };
        assert!(
            matches!(by_sys("event4")[..], [DeviceOp::SetTap(true), DeviceOp::SetNaturalScroll(true), DeviceOp::SetAccel(x)] if (x - 0.5).abs() < 1e-9)
        );
        assert!(matches!(
            by_sys("event6")[..],
            [DeviceOp::SetLeftHanded(true)]
        ));
        assert!(matches!(
            by_sys("event3")[..],
            [DeviceOp::SetDisabled(true)]
        ));

        // Metadata round-trips.
        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        let names: mlua::Table = t.get("names").unwrap();
        assert_eq!(names.get::<String>(1).unwrap(), "SynPS/2 Touchpad");
        assert_eq!(
            names.get::<String>(3).unwrap(),
            "AT Translated Set 2 keyboard"
        );
        let syses: mlua::Table = t.get("syses").unwrap();
        assert_eq!(syses.get::<String>(2).unwrap(), "event6");
    }

    #[test]
    fn set_accel_rejects_out_of_range_speed() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+D", function()
                neori.devices()[1]:set_accel(5.0)
            end)
            "#,
        )
        .config
        .unwrap();

        let mut ctx = FakeContext::with_devices(vec![DeviceSnapshot {
            name: "mouse".into(),
            sysname: "event1".into(),
            is_pointer: true,
            ..Default::default()
        }]);

        let err = with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap_err();
        assert!(err.contains("out of range"), "unexpected error: {err}");
        assert!(ctx.device_ops.is_empty());
    }

    #[test]
    fn pointer_and_keyboard_layout_api_returns_cached_values() {
        // neori.pointer() and neori.keyboard_layout() return nil when the caches
        // are unseeded, and reflect their contents once set.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = {}
            neori.on("startup", function()
                local p = neori.pointer()
                seen.pointer_nil = p == nil
                seen.layout_nil = neori.keyboard_layout() == nil
            end)
            neori.bind("Mod+Alt+P", function()
                local p = neori.pointer()
                if p then
                    seen.pointer_x = p.x
                    seen.pointer_y = p.y
                    seen.pointer_output = p.output
                end
                seen.layout = neori.keyboard_layout()
            end)
            "#,
        )
        .config
        .unwrap();

        // Cold: pointer() and keyboard_layout() should return nil.
        host.call_event("startup").unwrap();
        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        assert!(t.get::<bool>("pointer_nil").unwrap());
        assert!(t.get::<bool>("layout_nil").unwrap());

        let mut ctx = FakeContext {
            pointer: Some(PointerSnapshot {
                x: 42.5,
                y: 17.25,
                output_name: Some("eDP-1".into()),
            }),
            keyboard_layout: Some("English (US)".into()),
            ..Default::default()
        };
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();

        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        assert_eq!(t.get::<f64>("pointer_x").unwrap(), 42.5);
        assert_eq!(t.get::<f64>("pointer_y").unwrap(), 17.25);
        assert_eq!(t.get::<String>("pointer_output").unwrap(), "eDP-1");
        assert_eq!(t.get::<String>("layout").unwrap(), "English (US)");
    }

    #[test]
    fn warp_moves_the_cursor() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+Z", function()
                neori.warp(100, 200)
                neori.warp(300.5, 400.75)
            end)
            "#,
        )
        .config
        .unwrap();

        let mut ctx = FakeContext::default();
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(ctx.warps, vec![(100.0, 200.0), (300.5, 400.75)]);
    }

    #[test]
    fn warp_rejects_non_finite_coordinates() {
        // Guard: NaN / Inf should yield a Lua error, not move the cursor somewhere odd.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+X", function()
                neori.warp(1/0, 0)
            end)
            "#,
        )
        .config
        .unwrap();

        let mut ctx = FakeContext::default();
        let err = with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap_err();
        assert!(err.contains("neori.warp"), "unexpected error: {err}");
        assert!(ctx.warps.is_empty());
    }

    #[test]
    fn layout_changed_event_registers_and_dispatches() {
        // `layout-changed` takes the new layout name as its string argument.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = nil
            neori.on("layout-changed", function(name) seen = name end)
            "#,
        )
        .config
        .unwrap();

        host.call_event_with("layout-changed", "Russian".to_string())
            .unwrap();

        let seen = host.read_global("seen").unwrap();
        let mlua::Value::String(s) = seen else {
            panic!("seen is not a string: {seen:?}");
        };
        assert_eq!(s.to_str().unwrap().to_string(), "Russian");
    }

    #[test]
    fn window_opening_event_registers_and_dispatches() {
        // The `window-opening` event takes a `Window` payload (just like
        // `window-opened`). The handler should fire with the given snapshot.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = {}
            neori.on("window-opening", function(w)
                seen.app_id = w:app_id()
                seen.title  = w:title()
            end)
            "#,
        )
        .config
        .unwrap();

        let snap = WindowSnapshot {
            id: 0,
            app_id: Some("firefox".into()),
            title: Some("Example".into()),
            ..Default::default()
        };
        host.call_event_with("window-opening", Window(snap))
            .unwrap();

        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        assert_eq!(t.get::<String>("app_id").unwrap(), "firefox");
        assert_eq!(t.get::<String>("title").unwrap(), "Example");
    }

    #[test]
    fn open_overrides_write_only_inside_window_opening() {
        // `w:open_fullscreen(true)` inside a `window-opening` handler should
        // populate the overrides; the same call from a plain bind (no guard)
        // must be a silent no-op.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                -- No window-opening guard here — this must leak nothing.
                local w = neori.focused_window()
                if w then w:open_fullscreen(true) end
            end)
            neori.on("window-opening", function(w)
                w:open_fullscreen(true)
                w:open_on_workspace("browser")
                w:open_on_output("HDMI-A-1")
                w:open_maximized(true)
                w:open_floating(false)
            end)
            "#,
        )
        .config
        .unwrap();

        // 1. Bind path (no guard): no overrides should be recorded.
        let mut ctx = FakeContext::new(
            vec![WindowSnapshot {
                id: 42,
                ..Default::default()
            }],
            Some(42),
        );
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        host.begin_open_overrides();
        let after_bind = host.take_open_overrides();
        assert!(
            after_bind.open_fullscreen.is_none(),
            "no override should leak from the bind path: {after_bind:?}"
        );

        // 2. window-opening path: begin guard, fire, take.
        host.begin_open_overrides();
        host.call_event_with("window-opening", Window(WindowSnapshot::default()))
            .unwrap();
        let overrides = host.take_open_overrides();
        assert_eq!(overrides.open_fullscreen, Some(true));
        assert_eq!(overrides.open_on_workspace.as_deref(), Some("browser"));
        assert_eq!(overrides.open_on_output.as_deref(), Some("HDMI-A-1"));
        assert_eq!(overrides.open_maximized, Some(true));
        assert_eq!(overrides.open_floating, Some(false));
    }

    #[test]
    fn workspace_and_output_events_register_on_neori_on() {
        // The four new event names (`workspace-focused`, `workspace-added`,
        // `workspace-removed`, `output-focused`) should parse and dispatch a Lua
        // callback that receives the matching userdata.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = {}
            neori.on("workspace-focused", function(ws) seen.wf = ws:name() or "nil" end)
            neori.on("workspace-added",   function(ws) seen.wa = ws:id() end)
            neori.on("workspace-removed", function(ws) seen.wr = ws:id() end)
            neori.on("output-focused",    function(out) seen.of = out:name() end)
            "#,
        )
        .config
        .unwrap();

        let ws = WorkspaceSnapshot {
            idx: 1,
            name: Some("code".to_string()),
            id: 42,
            ..Default::default()
        };
        host.call_event_with("workspace-focused", LuaWorkspace(ws.clone()))
            .unwrap();
        host.call_event_with("workspace-added", LuaWorkspace(ws.clone()))
            .unwrap();
        host.call_event_with("workspace-removed", LuaWorkspace(ws))
            .unwrap();
        let out = OutputSnapshot {
            name: "eDP-1".to_string(),
            ..Default::default()
        };
        host.call_event_with("output-focused", LuaOutput(out))
            .unwrap();

        let seen = host.read_global("seen").unwrap();
        let mlua::Value::Table(t) = seen else {
            panic!("seen is not a table");
        };
        assert_eq!(t.get::<String>("wf").unwrap(), "code");
        assert_eq!(t.get::<u64>("wa").unwrap(), 42);
        assert_eq!(t.get::<u64>("wr").unwrap(), 42);
        assert_eq!(t.get::<String>("of").unwrap(), "eDP-1");
    }

    #[test]
    fn workspace_mutation_methods_run_actions() {
        // Each mutation method should run its matching Action.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+W", function()
                local ws = neori.focused_workspace()
                ws:focus()
                ws:send_to_output("HDMI-A-1")
                ws:set_name("code")
                ws:unset_name()
            end)
            "#,
        )
        .config
        .unwrap();

        let ws_snap = WorkspaceSnapshot {
            idx: 2,
            name: None,
            output_name: Some("eDP-1".to_string()),
            is_focused: true,
            id: 777,
            window_ids: vec![],
        };
        let id = ws_snap.id;
        let mut ctx = FakeContext::with_workspaces(vec![ws_snap], Some(id));

        let actions = run_bind(&host, &mut ctx);
        let kinds: Vec<&str> = actions
            .iter()
            .map(|a| match a {
                Action::FocusWorkspace { .. } => "focus",
                Action::MoveWorkspaceToMonitor { .. } => "to-monitor",
                Action::SetWorkspaceName { .. } => "set-name",
                Action::UnsetWorkspaceName { .. } => "unset-name",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["focus", "to-monitor", "set-name", "unset-name"],
            "unexpected actions: {kinds:?}"
        );

        for action in &actions {
            match action {
                Action::FocusWorkspace { reference } => {
                    assert!(matches!(
                        reference,
                        neori_ipc::WorkspaceReferenceArg::Id(777)
                    ));
                }
                Action::MoveWorkspaceToMonitor { output, reference } => {
                    assert_eq!(output, "HDMI-A-1");
                    assert!(matches!(
                        reference,
                        Some(neori_ipc::WorkspaceReferenceArg::Id(777))
                    ));
                }
                Action::SetWorkspaceName { name, workspace } => {
                    assert_eq!(name, "code");
                    assert!(matches!(
                        workspace,
                        Some(neori_ipc::WorkspaceReferenceArg::Id(777))
                    ));
                }
                Action::UnsetWorkspaceName { reference } => {
                    assert!(matches!(
                        reference,
                        Some(neori_ipc::WorkspaceReferenceArg::Id(777))
                    ));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn window_mutation_methods_run_actions() {
        // Every mutation method should run its matching Action; the fake context records
        // them so we can assert on them here.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                local w = neori.focused_window()
                w:close()
                w:focus()
                w:fullscreen(true)   -- not fullscreen, should run
                w:fullscreen(false)  -- already not fullscreen, no-op
                w:fullscreen()       -- always runs
                w:float(true)        -- not floating, runs MoveToFloating
                w:float(false)       -- already not floating, no-op
                w:send_to_workspace(2)
                w:send_to_workspace("browser")
                w:send_to_output("eDP-1")
            end)
            "#,
        )
        .config
        .unwrap();

        // The window is not fullscreen and not floating.
        let snap = WindowSnapshot {
            id: 42,
            is_fullscreen: false,
            is_floating: false,
            ..Default::default()
        };
        let mut ctx = FakeContext::new(vec![snap], Some(42));

        let actions = run_bind(&host, &mut ctx);
        let kinds: Vec<&str> = actions
            .iter()
            .map(|a| match a {
                Action::CloseWindow { .. } => "close",
                Action::FocusWindow { .. } => "focus",
                Action::FullscreenWindow { .. } => "fullscreen-toggle",
                Action::MoveWindowToFloating { .. } => "float-true",
                Action::MoveWindowToTiling { .. } => "float-false",
                Action::MoveWindowToWorkspace { reference, .. } => match reference {
                    neori_ipc::WorkspaceReferenceArg::Index(_) => "workspace-idx",
                    neori_ipc::WorkspaceReferenceArg::Name(_) => "workspace-name",
                    neori_ipc::WorkspaceReferenceArg::Id(_) => "workspace-id",
                },
                Action::MoveWindowToMonitor { .. } => "monitor",
                _ => "other",
            })
            .collect();

        // Expected: close, focus, fullscreen (set-true), fullscreen (no-arg toggle),
        // float-true, workspace-idx, workspace-name, monitor.
        // The two no-op calls (w:fullscreen(false), w:float(false)) must NOT run anything.
        assert_eq!(
            kinds,
            vec![
                "close",
                "focus",
                "fullscreen-toggle",
                "fullscreen-toggle",
                "float-true",
                "workspace-idx",
                "workspace-name",
                "monitor",
            ],
            "unexpected actions: {kinds:?}"
        );

        // Verify ids / args carry through.
        for action in &actions {
            match action {
                Action::CloseWindow { id } => assert_eq!(*id, Some(42)),
                Action::FocusWindow { id } => assert_eq!(*id, 42),
                Action::FullscreenWindow { id } => assert_eq!(*id, Some(42)),
                Action::MoveWindowToFloating { id } => assert_eq!(*id, Some(42)),
                Action::MoveWindowToTiling { id } => assert_eq!(*id, Some(42)),
                Action::MoveWindowToWorkspace {
                    window_id,
                    reference,
                    focus,
                } => {
                    assert_eq!(*window_id, Some(42));
                    assert!(*focus);
                    match reference {
                        neori_ipc::WorkspaceReferenceArg::Index(2) => {}
                        neori_ipc::WorkspaceReferenceArg::Name(name) if name == "browser" => {}
                        other => panic!("unexpected reference {other:?}"),
                    };
                }
                Action::MoveWindowToMonitor { id, output } => {
                    assert_eq!(*id, Some(42));
                    assert_eq!(output, "eDP-1");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn window_mutation_on_stale_id_still_runs() {
        // A Lua config can stash a Window in a local and keep calling methods after the
        // window is gone. The method must not error — the compositor ignores actions
        // with unknown ids.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                stashed = stashed or neori.focused_window()
                stashed:close()
            end)
            "#,
        )
        .config
        .unwrap();

        let snap = WindowSnapshot {
            id: 99,
            ..Default::default()
        };
        let mut ctx = FakeContext::new(vec![snap], Some(99));

        let actions = run_bind(&host, &mut ctx);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::CloseWindow { id: Some(99) }));

        // Second call: the window is gone, but Lua `stashed` still holds the Window
        // userdata. Call it again — must still run the same action without erroring.
        let mut ctx = FakeContext::default();
        let actions = run_bind(&host, &mut ctx);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::CloseWindow { id: Some(99) }));
    }

    #[test]
    fn window_api_registers_on_neori_table() {
        // neori.windows(), neori.focused_window(), and the three new events are on
        // the neori API, not via parse_bind_entry — this test just verifies they parse.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("window-opened",  function(w) end)
            neori.on("window-closed",  function(w) end)
            neori.on("window-focused", function(w) end)
            local n = #neori.windows()
            local f = neori.focused_window()
            "#,
        )
        .config
        .unwrap();

        // The handlers are registered under their canonical event names.
        for ev in ["window-opened", "window-closed", "window-focused"] {
            // The registration was successful if the call doesn't error.
            host.call_event_with::<Window>(ev, Window(WindowSnapshot::default()))
                .unwrap();
        }
    }

    #[test]
    fn window_userdata_methods_return_snapshot_fields() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            captured = {}
            neori.on("window-focused", function(w)
                captured.id         = w:id()
                captured.app_id     = w:app_id()
                captured.title      = w:title()
                captured.focused    = w:is_focused()
                captured.floating   = w:is_floating()
                captured.fullscreen = w:is_fullscreen()
                captured.output     = w:output()
                captured.ws_idx     = w:workspace() and w:workspace().idx or nil
                captured.ws_name    = w:workspace() and w:workspace().name or nil
            end)
            "#,
        )
        .config
        .unwrap();

        let snap = WindowSnapshot {
            id: 42,
            app_id: Some("firefox".to_string()),
            title: Some("Example — Mozilla Firefox".to_string()),
            is_focused: true,
            is_floating: false,
            is_fullscreen: false,
            is_urgent: false,
            is_maximized: false,
            pid: Some(1234),
            workspace_idx: Some(3),
            workspace_name: Some("browser".to_string()),
            output_name: Some("eDP-1".to_string()),
        };
        host.call_event_with("window-focused", Window(snap))
            .unwrap();

        let captured = host
            .read_global("captured")
            .expect("captured table missing");
        let mlua::Value::Table(t) = captured else {
            panic!("captured is not a table");
        };
        assert_eq!(t.get::<u64>("id").unwrap(), 42);
        assert_eq!(t.get::<String>("app_id").unwrap(), "firefox");
        assert_eq!(
            t.get::<String>("title").unwrap(),
            "Example — Mozilla Firefox"
        );
        assert!(t.get::<bool>("focused").unwrap());
        assert!(!t.get::<bool>("floating").unwrap());
        assert!(!t.get::<bool>("fullscreen").unwrap());
        assert_eq!(t.get::<String>("output").unwrap(), "eDP-1");
        assert_eq!(t.get::<u64>("ws_idx").unwrap(), 3);
        assert_eq!(t.get::<String>("ws_name").unwrap(), "browser");
    }

    #[test]
    fn stashed_window_reads_live_state() {
        // A Window kept in a Lua variable reads the window's current state on every call,
        // and falls back to the last state it saw once the window is gone.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                stashed = stashed or neori.focused_window()
                title = stashed:title()
                fullscreen = stashed:is_fullscreen()
                label = tostring(stashed)
            end)
            "#,
        )
        .config
        .unwrap();
        let read_string = |name: &str| match host.read_global(name).unwrap() {
            mlua::Value::String(s) => s.to_str().unwrap().to_string(),
            other => panic!("{name} is {other:?}"),
        };

        let window = |title: &str, fullscreen| WindowSnapshot {
            id: 7,
            title: Some(title.into()),
            is_fullscreen: fullscreen,
            ..Default::default()
        };

        let mut ctx = FakeContext::new(vec![window("first", false)], Some(7));
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(read_string("title"), "first");

        // The window changed since: the stashed Window sees it.
        let mut ctx = FakeContext::new(vec![window("second", true)], None);
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(read_string("title"), "second");
        assert!(matches!(
            host.read_global("fullscreen").unwrap(),
            mlua::Value::Boolean(true)
        ));
        assert!(read_string("label").contains("second"));

        // The window is gone: reads fall back to what the Window was created with.
        let mut ctx = FakeContext::default();
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(read_string("title"), "first");
    }

    #[test]
    fn fullscreen_toggle_checks_live_state() {
        // w:fullscreen(true) must look at the window's current state, not at the state
        // from when the Window was created.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                stashed = stashed or neori.focused_window()
                stashed:fullscreen(true)
            end)
            "#,
        )
        .config
        .unwrap();
        let window = |fullscreen| WindowSnapshot {
            id: 3,
            is_fullscreen: fullscreen,
            ..Default::default()
        };

        let mut ctx = FakeContext::new(vec![window(false)], Some(3));
        let actions = run_bind(&host, &mut ctx);
        assert_eq!(actions, [Action::FullscreenWindow { id: Some(3) }]);

        // Now it's fullscreen: setting it again is a no-op.
        let mut ctx = FakeContext::new(vec![window(true)], Some(3));
        let actions = run_bind(&host, &mut ctx);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn windows_are_empty_without_a_callback_context() {
        // While the config itself is evaluated there's no compositor context.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            count_at_parse = #neori.windows()
            focused_at_parse = neori.focused_window()
            "#,
        )
        .config
        .unwrap();
        assert!(matches!(
            host.read_global("count_at_parse").unwrap(),
            mlua::Value::Integer(0)
        ));
        assert!(matches!(
            host.read_global("focused_at_parse").unwrap(),
            mlua::Value::Nil
        ));
    }

    #[test]
    fn stashed_output_workspace_and_device_read_live_state() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                out = out or neori.focused_output()
                ws = ws or neori.focused_workspace()
                dev = dev or neori.devices()[1]
                scale = out:scale()
                off = out:is_off()
                ws_name = ws:name()
                ws_idx = ws:idx()
                dev_name = dev:name()
            end)
            "#,
        )
        .config
        .unwrap();
        let read = |name: &str| host.read_global(name).unwrap();
        let ctx = |scale, ws_name: &str, idx, dev_name: &str| FakeContext {
            outputs: vec![OutputSnapshot {
                name: "eDP-1".into(),
                scale: Some(scale),
                ..Default::default()
            }],
            focused_output: Some("eDP-1".into()),
            workspaces: vec![WorkspaceSnapshot {
                id: 5,
                idx,
                name: Some(ws_name.into()),
                ..Default::default()
            }],
            focused_workspace: Some(5),
            devices: vec![DeviceSnapshot {
                sysname: "event4".into(),
                name: dev_name.into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let mut first = ctx(1.0, "web", 1, "old");
        with_script_context(&mut first, || host.call_bind_function(1)).unwrap();
        assert!(matches!(read("scale"), mlua::Value::Number(s) if s == 1.0));

        let mut second = ctx(2.0, "chat", 3, "new");
        with_script_context(&mut second, || host.call_bind_function(1)).unwrap();
        assert!(matches!(read("scale"), mlua::Value::Number(s) if s == 2.0));
        assert!(matches!(read("off"), mlua::Value::Boolean(false)));
        assert!(matches!(read("ws_idx"), mlua::Value::Integer(3)));
        let mlua::Value::String(ws_name) = read("ws_name") else {
            panic!("ws_name is not a string");
        };
        assert_eq!(ws_name.to_str().unwrap(), "chat");
        let mlua::Value::String(dev_name) = read("dev_name") else {
            panic!("dev_name is not a string");
        };
        assert_eq!(dev_name.to_str().unwrap(), "new");

        // The output is no longer listed (turned off or unplugged): it reads as off, and the
        // other objects keep their last known state.
        let mut gone = FakeContext::default();
        with_script_context(&mut gone, || host.call_bind_function(1)).unwrap();
        assert!(matches!(read("off"), mlua::Value::Boolean(true)));
        assert!(matches!(read("scale"), mlua::Value::Number(s) if s == 1.0));
        assert!(matches!(read("ws_idx"), mlua::Value::Integer(1)));
    }

    #[test]
    fn output_enable_checks_live_state() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                out = out or neori.focused_output()
                out:enable(true)
            end)
            "#,
        )
        .config
        .unwrap();
        let output = || OutputSnapshot {
            name: "eDP-1".into(),
            ..Default::default()
        };

        // On: enable(true) is a no-op.
        let mut ctx = FakeContext::with_outputs(vec![output()], Some("eDP-1"));
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert!(ctx.output_ops.is_empty());

        // Turned off since: enable(true) turns it back on.
        let mut ctx = FakeContext::default();
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        let ops = std::mem::take(&mut ctx.output_ops);
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0].0, "eDP-1");
        assert!(matches!(ops[0].1, neori_ipc::OutputAction::On));
    }

    #[test]
    fn queries_are_empty_without_a_callback_context() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            results = {
                outputs = #neori.outputs(),
                workspaces = #neori.workspaces(),
                devices = #neori.devices(),
                focused_output = neori.focused_output() == nil,
                focused_workspace = neori.focused_workspace() == nil,
                pointer = neori.pointer() == nil,
                layout = neori.keyboard_layout() == nil,
            }
            "#,
        )
        .config
        .unwrap();
        let mlua::Value::Table(results) = host.read_global("results").unwrap() else {
            panic!("results is not a table");
        };
        for key in ["outputs", "workspaces", "devices"] {
            assert_eq!(results.get::<i64>(key).unwrap(), 0, "{key}");
        }
        for key in ["focused_output", "focused_workspace", "pointer", "layout"] {
            assert!(results.get::<bool>(key).unwrap(), "{key}");
        }
    }

    fn int_global(host: &ScriptHost, name: &str) -> i64 {
        match host.read_global(name).unwrap() {
            mlua::Value::Integer(n) => n,
            other => panic!("{name} is {other:?}"),
        }
    }

    #[test]
    fn custom_global_signals() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            total = 0
            local add = neori.connect("my-signal", function(n) total = total + n end)
            neori.connect("my-signal", function(n) total = total + 10 * n end)
            neori.emit("my-signal", 1)
            after_emit = total

            -- connect returns the function, so it can be disconnected.
            removed_one = neori.disconnect("my-signal", add)
            removed_again = neori.disconnect("my-signal", add)
            neori.emit("my-signal", 1)
            after_disconnect = total

            removed_all = neori.disconnect("my-signal")
            neori.emit("my-signal", 1)
            after_disconnect_all = total
            "#,
        )
        .config
        .unwrap();
        assert_eq!(int_global(&host, "after_emit"), 11);
        assert_eq!(int_global(&host, "after_disconnect"), 21);
        assert_eq!(int_global(&host, "after_disconnect_all"), 21);
        let bool_global = |name| match host.read_global(name).unwrap() {
            mlua::Value::Boolean(b) => b,
            other => panic!("{name} is {other:?}"),
        };
        assert!(bool_global("removed_one"));
        assert!(!bool_global("removed_again"));
        assert!(bool_global("removed_all"));
    }

    #[test]
    fn one_shot_handler_disconnects_itself() {
        // The pattern from the wiki: a handler that disconnects itself, leaving the others.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            once, always = 0, 0
            local first_open
            first_open = neori.connect("window-opened", function(w)
                once = once + 1
                neori.disconnect("window-opened", first_open)
            end)
            neori.on("window-opened", function(w) always = always + 1 end)
            "#,
        )
        .config
        .unwrap();
        for _ in 0..3 {
            host.call_event_with("window-opened", Window(WindowSnapshot::default()))
                .unwrap();
        }
        assert_eq!(int_global(&host, "once"), 1);
        assert_eq!(int_global(&host, "always"), 3);
    }

    /// Renders `neori.signals()` / `obj:signals()` rows as "name:handlers" strings, with
    /// "+objects" for global built-ins that also reach objects, and "*" for custom signals.
    fn signal_rows(host: &ScriptHost, global: &str) -> Vec<String> {
        let mlua::Value::Table(list) = host.read_global(global).unwrap() else {
            panic!("{global} is not a table");
        };
        list.sequence_values::<mlua::Table>()
            .map(|row| {
                let row = row.unwrap();
                let name: String = row.get("name").unwrap();
                let handlers: i64 = row.get("handlers").unwrap();
                let builtin: bool = row.get("builtin").unwrap();
                if builtin {
                    let description: String = row.get("description").unwrap();
                    let args: String = row.get("args").unwrap();
                    assert!(!description.is_empty() && !args.is_empty(), "{name}");
                }
                let mut s = format!("{name}:{handlers}");
                if let Some(n) = row.get::<Option<i64>>("object_handlers").unwrap() {
                    let short: String = row.get("object_signal").unwrap();
                    s += &format!("+{short}:{n}");
                }
                if !builtin {
                    s += "*";
                }
                s
            })
            .collect()
    }

    #[test]
    fn neori_signals_lists_builtins_and_connected_custom_signals() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("window-closed", function() end)
            neori.connect("window_closed", function() end)
            neori.connect("zeta", function() end)
            neori.connect("alpha", function() end)
            neori.connect("alpha", function() end)
            neori.bind("Mod+Alt+T", function()
                neori.focused_window():connect("closed", function() end)
                neori.focused_window():connect("poke", function() end)
                list = neori.signals()
                window_list = neori.focused_window():signals()
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext::new(
            vec![WindowSnapshot {
                id: 4,
                ..Default::default()
            }],
            Some(4),
        );
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();

        let rows = signal_rows(&host, "list");
        // Every built-in signal is listed, in order, even without handlers.
        let builtin_names: Vec<_> = crate::signals::BUILTIN_SIGNALS
            .iter()
            .map(|s| s.name)
            .collect();
        let listed: Vec<_> = rows
            .iter()
            .take(builtin_names.len())
            .map(|r| r.split(':').next().unwrap())
            .collect();
        assert_eq!(listed, builtin_names);
        assert!(
            rows.contains(&"window-closed:2+closed:1".to_string()),
            "{rows:?}"
        );
        assert!(rows.contains(&"window-opened:0".to_string()), "{rows:?}");
        assert!(
            rows.contains(&"workspace-focused:0+focused:0".to_string()),
            "{rows:?}"
        );
        // Custom signals with handlers come last, sorted. The window's "poke" isn't global.
        assert_eq!(rows[builtin_names.len()..], ["alpha:2*", "zeta:1*"]);

        // The window's own list: its built-in signals, then its custom ones.
        assert_eq!(
            signal_rows(&host, "window_list"),
            [
                "closed:1",
                "focused:0",
                "title-changed:0",
                "fullscreen-changed:0",
                "floating-changed:0",
                "urgent-changed:0",
                "workspace-changed:0",
                "poke:1*"
            ]
        );
    }

    #[test]
    fn object_signal_lists_per_kind() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                ws_list = neori.focused_workspace():signals()
                out_list = neori.focused_output():signals()
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext {
            outputs: vec![OutputSnapshot {
                name: "DP-1".into(),
                ..Default::default()
            }],
            focused_output: Some("DP-1".into()),
            workspaces: vec![WorkspaceSnapshot {
                id: 3,
                ..Default::default()
            }],
            focused_workspace: Some(3),
            ..Default::default()
        };
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(
            signal_rows(&host, "ws_list"),
            ["focused:0", "removed:0", "name-changed:0"]
        );
        assert_eq!(
            signal_rows(&host, "out_list"),
            ["added:0", "removed:0", "focused:0", "mode-changed:0"]
        );
    }

    #[test]
    fn on_and_connect_share_builtin_signals() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            calls = 0
            neori.on("layout_changed", function(name) calls = calls + 1 end)
            neori.connect("layout-changed", function(name) calls = calls + 1 end)
            "#,
        )
        .config
        .unwrap();
        host.call_event_with("layout-changed", "us".to_string())
            .unwrap();
        assert_eq!(int_global(&host, "calls"), 2);
    }

    #[test]
    fn on_still_rejects_unknown_events() {
        let err = Config::parse_mem(r#"neori.on("window-cloesd", function() end)"#).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("unknown event 'window-cloesd'"), "{msg}");
        assert!(msg.contains("neori.connect"), "{msg}");
    }

    #[test]
    fn emit_runs_every_handler_and_raises_the_first_error() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            ran = 0
            neori.connect("x", function() ran = ran + 1; error("first") end)
            neori.connect("x", function() ran = ran + 1; error("second") end)
            neori.connect("x", function() ran = ran + 1 end)
            ok, err = pcall(neori.emit, "x")
            err = tostring(err)
            "#,
        )
        .config
        .unwrap();
        assert_eq!(int_global(&host, "ran"), 3);
        assert!(matches!(
            host.read_global("ok").unwrap(),
            mlua::Value::Boolean(false)
        ));
        let mlua::Value::String(err) = host.read_global("err").unwrap() else {
            panic!("err is not a string");
        };
        let err = err.to_str().unwrap().to_string();
        assert!(err.contains("first") && !err.contains("second"), "{err}");
    }

    #[test]
    fn handlers_may_connect_while_a_signal_runs() {
        // The registry must not be locked while handlers run.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            late = 0
            neori.connect("x", function()
                neori.connect("x", function() late = late + 1 end)
                neori.disconnect("y")
            end)
            neori.emit("x")
            neori.emit("x")
            "#,
        )
        .config
        .unwrap();
        // The handler added by the first emit runs on the second one only.
        assert_eq!(int_global(&host, "late"), 1);
    }

    #[test]
    fn window_signals() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            closed = 0
            custom = 0
            neori.bind("Mod+Alt+T", function()
                local w = neori.focused_window()
                w:connect("closed", function(win) closed = closed + win:id() end)
                w:connect("poke", function(win, n) custom = custom + win:id() * n end)
                w:emit("poke", 2)
                -- Another window doesn't see this window's signals.
                neori.windows()[2]:emit("poke", 100)
            end)
            "#,
        )
        .config
        .unwrap();
        let windows = vec![
            WindowSnapshot {
                id: 7,
                ..Default::default()
            },
            WindowSnapshot {
                id: 8,
                ..Default::default()
            },
        ];
        let mut ctx = FakeContext::new(windows, Some(7));
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();
        assert_eq!(int_global(&host, "custom"), 14);

        // A built-in event about the window reaches its own handlers, then drops them.
        let target = SignalTarget::Window(7);
        let window = Window(WindowSnapshot {
            id: 7,
            ..Default::default()
        });
        let call = host.event_call("window-closed", Some(&target)).unwrap();
        call.run(window).unwrap();
        assert_eq!(int_global(&host, "closed"), 7);
        assert!(host.event_call("window-closed", Some(&target)).is_none());

        // Events about other windows don't reach it.
        assert!(host
            .event_call("window-closed", Some(&SignalTarget::Window(8)))
            .is_none());
    }

    #[test]
    fn workspace_and_output_signals() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            ws_focused = 0
            out_removed = 0
            neori.bind("Mod+Alt+T", function()
                neori.focused_workspace():connect("focused", function(ws)
                    ws_focused = ws_focused + 1
                end)
                neori.focused_output():connect("removed", function(connector, out)
                    out_removed = out_removed + 1
                end)
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext {
            outputs: vec![OutputSnapshot {
                name: "DP-1".into(),
                ..Default::default()
            }],
            focused_output: Some("DP-1".into()),
            workspaces: vec![WorkspaceSnapshot {
                id: 3,
                ..Default::default()
            }],
            focused_workspace: Some(3),
            ..Default::default()
        };
        with_script_context(&mut ctx, || host.call_bind_function(1)).unwrap();

        let ws = SignalTarget::Workspace(3);
        let ws_value = LuaWorkspace(WorkspaceSnapshot {
            id: 3,
            ..Default::default()
        });
        for _ in 0..2 {
            let call = host.event_call("workspace-focused", Some(&ws)).unwrap();
            call.run(ws_value.clone()).unwrap();
        }
        assert_eq!(int_global(&host, "ws_focused"), 2);

        // Output handlers are kept across monitor-removed: the connector can come back.
        let out = SignalTarget::Output("DP-1".into());
        let out_value = LuaOutput(OutputSnapshot {
            name: "DP-1".into(),
            ..Default::default()
        });
        for _ in 0..2 {
            let call = host.event_call("monitor-removed", Some(&out)).unwrap();
            call.run(("DP-1".to_string(), out_value.clone())).unwrap();
        }
        assert_eq!(int_global(&host, "out_removed"), 2);
    }

    #[test]
    fn window_without_id_cannot_connect() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.on("window-opening", function(w)
                ok, err = pcall(function() w:connect("closed", function() end) end)
                err = tostring(err)
            end)
            "#,
        )
        .config
        .unwrap();
        host.call_event_with("window-opening", Window(WindowSnapshot::default()))
            .unwrap();
        assert!(matches!(
            host.read_global("ok").unwrap(),
            mlua::Value::Boolean(false)
        ));
        let mlua::Value::String(err) = host.read_global("err").unwrap() else {
            panic!("err is not a string");
        };
        assert!(err.to_str().unwrap().contains("window-opened"));
    }

    #[test]
    fn changes_outside_a_callback_are_an_error() {
        // While the config file is evaluated there is no compositor to apply changes to.
        for lua in [r#"neori.action("close-window")"#, r#"neori.warp(1, 2)"#] {
            let err = Config::parse_mem(lua).unwrap_err();
            let msg = format!("{err:?}");
            assert!(msg.contains("only work inside a callback"), "{lua}: {msg}");
        }
    }

    #[test]
    fn changes_are_applied_right_away_in_order() {
        // Each change reaches the compositor when it's made, interleaved with reads.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                neori.action("focus-column-left")
                neori.warp(1, 2)
                neori.focused_output():set_scale(2)
                neori.devices()[1]:set_tap(true)
                neori.action("focus-column-right")
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext {
            outputs: vec![OutputSnapshot {
                name: "DP-1".into(),
                ..Default::default()
            }],
            focused_output: Some("DP-1".into()),
            devices: vec![DeviceSnapshot {
                sysname: "event1".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let actions = run_bind(&host, &mut ctx);
        assert_eq!(
            actions,
            [Action::FocusColumnLeft {}, Action::FocusColumnRight {}]
        );
        assert_eq!(ctx.warps, [(1.0, 2.0)]);
        assert_eq!(ctx.output_ops.len(), 1);
        assert_eq!(ctx.device_ops.len(), 1);
    }

    #[test]
    fn api_functions_cant_be_assigned() {
        // `neori.prefer_no_csd = true` would replace the function and set nothing.
        let msg = format!(
            "{:?}",
            Config::parse_mem("neori.prefer_no_csd = true").unwrap_err()
        );
        assert!(
            msg.contains("neori.prefer_no_csd is a function and can't be assigned"),
            "{msg}"
        );
        assert!(msg.contains("neori.prefer_no_csd(...)"), "{msg}");

        // New fields are fine, for sharing code between included files, and the API keeps
        // working and listing itself.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.my_helper = function() return 42 end
            helper_result = neori.my_helper()
            neori.prefer_no_csd(true)
            has_bind = false
            for name in pairs(neori) do
                if name == "bind" then has_bind = true end
            end
            "#,
        )
        .config
        .unwrap();
        assert!(matches!(
            host.read_global("helper_result").unwrap(),
            mlua::Value::Integer(42)
        ));
        assert!(matches!(
            host.read_global("has_bind").unwrap(),
            mlua::Value::Boolean(true)
        ));
    }

    #[test]
    fn config_sections_are_namespaces() {
        let (config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            unset_gaps = neori.layout.gaps            -- nil: nothing set yet
            neori.layout { gaps = 3, focus_ring = { width = 2 } }
            after_call = neori.layout.gaps            -- 3
            neori.layout.gaps = 7                     -- one field
            neori.layout.focus_ring.width = 5         -- nested, the subsection exists
            after_assign = neori.layout.gaps          -- 7
            neori.cursor.xcursor_size = 32            -- creates the section
            neori.input = { keyboard = { numlock = true } }  -- whole section, like a call
            fields = 0
            for _ in pairs(neori.layout) do fields = fields + 1 end
            neori.bind("Mod+T", function() at_runtime = neori.layout.gaps end)
            "#,
        )
        .config
        .unwrap();
        assert_eq!(config.layout.gaps, 7.0);
        assert_eq!(config.layout.focus_ring.width, 5.0);
        assert_eq!(config.cursor.xcursor_size, 32);
        assert!(config.input.keyboard.numlock);

        let int = |name: &str| match host.read_global(name).unwrap() {
            mlua::Value::Integer(n) => n,
            other => panic!("{name} is {other:?}"),
        };
        assert!(matches!(
            host.read_global("unset_gaps").unwrap(),
            mlua::Value::Nil
        ));
        assert_eq!(int("after_call"), 3);
        assert_eq!(int("after_assign"), 7);
        assert_eq!(int("fields"), 2);

        // Inside a callback, a read gives what the loaded config set.
        run_bind(&host, &mut FakeContext::default());
        assert_eq!(int("at_runtime"), 7);
    }

    #[test]
    fn section_calls_merge() {
        let config = Config::parse_mem(
            r#"
            neori.layout {
                gaps = 3,
                focus_ring = { width = 2, active_color = "blue" },
                preset_column_widths = { { proportion = 0.25 }, { proportion = 0.5 } },
                struts = { left = 64, right = 64 },
            }
            neori.layout {
                center_focused_column = "always",
                focus_ring = { width = 5 },                  -- merges: the color stays
                preset_column_widths = { { proportion = 1.0 } },  -- lists replace
                struts = { top = 32 },                       -- struts replace
            }
            neori.input { keyboard = { numlock = true }, touchpad = { tap = true } }
            neori.input { keyboard = { repeat_rate = 40 }, touchpad = { natural_scroll = true } }
            neori.environment { A = "1" }
            neori.environment { B = "2" }
            "#,
        )
        .unwrap();
        let layout = &config.layout;
        assert_eq!(layout.gaps, 3.0);
        assert_eq!(layout.center_focused_column, CenterFocusedColumn::Always);
        assert_eq!(layout.focus_ring.width, 5.0);
        assert_eq!(
            layout.focus_ring.active_color,
            Color::from_rgba8_unpremul(0, 0, 255, 255)
        );
        assert_eq!(layout.preset_column_widths, [PresetSize::Proportion(1.0)]);
        assert_eq!(layout.struts.left.0, 0.0);
        assert_eq!(layout.struts.top.0, 32.0);

        // Keyboard settings merge; a pointing device section is replaced.
        assert!(config.input.keyboard.numlock);
        assert_eq!(config.input.keyboard.repeat_rate, 40);
        assert!(!config.input.touchpad.tap);
        assert!(config.input.touchpad.natural_scroll);

        let mut env: Vec<_> = config
            .environment
            .0
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        env.sort();
        assert_eq!(env, ["A", "B"]);
    }

    #[test]
    fn section_merge_details() {
        let (config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            -- A color replaces a gradient set before it.
            neori.layout { focus_ring = { active_gradient = { from = "red", to = "blue" } } }
            neori.layout { focus_ring = { active_color = "green" } }
            -- Each animation is replaced as a whole.
            neori.animations { window_open = { duration_ms = 100, curve = "linear" } }
            neori.animations { window_open = { spring = { damping_ratio = 1.0, stiffness = 800, epsilon = 0.0001 } } }
            -- The caller's tables aren't changed by later merges.
            shared = { width = 1 }
            neori.layout { border = shared }
            neori.layout { border = { width = 9 } }
            shared_width = shared.width
            "#,
        )
        .config
        .unwrap();
        assert!(config.layout.focus_ring.active_gradient.is_none());
        assert_eq!(
            config.layout.focus_ring.active_color,
            Color::from_rgba8_unpremul(0, 128, 0, 255)
        );
        assert!(matches!(
            config.animations.window_open.anim.kind,
            crate::animations::Kind::Spring(_)
        ));
        assert_eq!(config.layout.border.width, 9.0);
        assert!(matches!(
            host.read_global("shared_width").unwrap(),
            mlua::Value::Integer(1)
        ));
    }

    #[test]
    fn section_merge_across_includes() {
        // The examples from the Include wiki page.
        let dir = std::env::temp_dir().join(format!("neori-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("colors.lua"),
            r#"neori.layout { focus_ring = { active_color = "blue" }, border = { active_color = "green" } }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("struts.lua"),
            "neori.layout { struts = { left = 64, right = 64 } }",
        )
        .unwrap();
        let config_path = dir.join("config.lua");
        let text = r#"
            neori.include("colors.lua")
            neori.layout { gaps = 8, border = { width = 8 } }
            neori.layout { struts = { top = 64, bottom = 64 } }
            neori.include("struts.lua")
        "#;
        let config = Config::parse(&config_path, text).config.unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let layout = &config.layout;
        assert_eq!(layout.gaps, 8.0);
        assert_eq!(layout.border.width, 8.0);
        assert_eq!(
            layout.border.active_color,
            Color::from_rgba8_unpremul(0, 128, 0, 255)
        );
        assert_eq!(
            layout.focus_ring.active_color,
            Color::from_rgba8_unpremul(0, 0, 255, 255)
        );
        // Struts are replaced: only left and right remain.
        assert_eq!(
            (
                layout.struts.left.0,
                layout.struts.right.0,
                layout.struts.top.0
            ),
            (64.0, 64.0, 0.0)
        );
    }

    #[test]
    fn neori_misc() {
        let (config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.misc.prefer_no_csd = true
            csd = neori.misc.prefer_no_csd
            neori.misc.screenshot_path = nil          -- don't save screenshots
            path = neori.misc.screenshot_path         -- false
            "#,
        )
        .config
        .unwrap();
        assert!(config.prefer_no_csd);
        assert_eq!(config.screenshot_path.0, None);
        assert!(matches!(
            host.read_global("csd").unwrap(),
            mlua::Value::Boolean(true)
        ));
        assert!(matches!(
            host.read_global("path").unwrap(),
            mlua::Value::Boolean(false)
        ));

        // Callable with a table; the function forms set the same options.
        let config = Config::parse_mem(
            r#"
            neori.misc { screenshot_path = "~/a.png" }
            neori.prefer_no_csd(true)
            assert(neori.misc.prefer_no_csd == true)
            "#,
        )
        .unwrap();
        assert_eq!(config.screenshot_path.0.as_deref(), Some("~/a.png"));
        assert!(config.prefer_no_csd);

        let msg = format!(
            "{:?}",
            Config::parse_mem("neori.misc.prefer_no_cds = true").unwrap_err()
        );
        assert!(
            msg.contains("neori.misc has no option prefer_no_cds"),
            "{msg}"
        );
    }

    #[test]
    fn false_disables_screenshots_and_unsets_environment_variables() {
        let config = Config::parse_mem(
            r#"
            neori.screenshot_path(false)
            neori.environment { DISPLAY = false, QT_QPA_PLATFORM = "wayland" }
            "#,
        )
        .unwrap();
        assert_eq!(config.screenshot_path.0, None);
        let mut env: Vec<_> = config
            .environment
            .0
            .iter()
            .map(|v| (v.name.as_str(), v.value.as_deref()))
            .collect();
        env.sort();
        assert_eq!(
            env,
            [("DISPLAY", None), ("QT_QPA_PLATFORM", Some("wayland"))]
        );

        let config = Config::parse_mem(r#"neori.screenshot_path("~/shot.png")"#).unwrap();
        assert_eq!(config.screenshot_path.0.as_deref(), Some("~/shot.png"));

        // `true` isn't a path: a warning, or an error in strict mode.
        let msg = format!(
            "{:?}",
            Config::parse_mem("neori.strict(true) neori.screenshot_path(true)").unwrap_err()
        );
        assert!(msg.contains("screenshot_path"), "{msg}");
    }

    #[test]
    fn config_of_a_single_expression_parses() {
        // Only an explicit `return` is the config's result: a config that is a single call
        // returning a value must not have that value taken as its config table.
        for lua in [
            r#"neori.connect("x", function() end)"#,
            r#"neori.spawn("kitty")"#,
            r#"neori.after(1000, function() end)"#,
        ] {
            assert!(Config::parse_mem(lua).is_ok(), "{lua}");
        }
        let config = Config::parse_mem("return { layout = { gaps = 7 } }").unwrap();
        assert_eq!(config.layout.gaps, 7.0);
    }

    #[test]
    fn eval_lua_code() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            from_config = "set by the config"
            neori.on("startup", function() end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext::new(
            vec![WindowSnapshot {
                id: 4,
                title: Some("Terminal".into()),
                ..Default::default()
            }],
            Some(4),
        );
        let mut eval = |code: &str| with_script_context(&mut ctx, || host.eval(code)?.run());
        let values = |code: &str, eval: &mut dyn FnMut(&str) -> Result<_, String>| {
            let out: parse_lua::LuaEvalOutput = eval(code).unwrap();
            out.values
        };

        // Expressions, with every value they return.
        assert_eq!(values("1 + 1", &mut eval), ["2"]);
        assert_eq!(
            values("1, 'two', nil, true", &mut eval),
            ["1", "two", "nil", "true"]
        );
        assert_eq!(values("from_config", &mut eval), ["set by the config"]);
        assert_eq!(
            values("neori.focused_window():title()", &mut eval),
            ["Terminal"]
        );
        assert_eq!(
            values("neori.focused_window()", &mut eval),
            [r#"Window(#4, app_id="?", title="Terminal")"#]
        );
        // Tables: the list part, then the fields sorted; strings quoted inside.
        assert_eq!(
            values("{ 'a', 2, z = { x = 1 }, b = 'c' }", &mut eval),
            [r#"{ "a", 2, b = "c", z = { x = 1 } }"#]
        );
        assert_eq!(values("{}", &mut eval), ["{}"]);
        assert_eq!(
            values("{ { { { { 1 } } } } }", &mut eval),
            ["{ { { { { … 1 entry } } } } }"]
        );
        let long = values(
            "(function() local t = {} for i = 1, 150 do t[i] = i end return t end)()",
            &mut eval,
        );
        assert!(long[0].ends_with(", 100, … }"), "{}", long[0]);

        // Statements, which can return values; prints are collected, in order.
        let out = eval("print('one') local x = 20 print('two', x) return x + 1").unwrap();
        assert_eq!(out.printed, "one\ntwo\t20\n");
        assert_eq!(out.values, ["21"]);
        let out = eval("local y = 1").unwrap();
        assert!(out.values.is_empty() && out.printed.is_empty());

        // Errors, with what was printed before them.
        let err = eval("print('before') error('boom')").unwrap_err();
        assert!(err.starts_with("before\n") && err.contains("boom"), "{err}");
        let err = eval("this is not lua").unwrap_err();
        assert!(err.contains("syntax error"), "{err}");

        // Prints go back to the log once the code is done.
        let out = eval("return 1").unwrap();
        assert!(out.printed.is_empty());
    }

    #[test]
    fn is_startup() {
        let lua = r#"
            while_loading = neori.is_startup()
            neori.bind("Mod+T", function() in_callback = neori.is_startup() end)
        "#;
        let read = |host: &ScriptHost, name: &str| match host.read_global(name).unwrap() {
            mlua::Value::Boolean(b) => b,
            other => panic!("{name} is {other:?}"),
        };

        // The compositor's first config: starting up until the startup handlers ran.
        let (_config, host) = Config::parse_with_host(std::path::Path::new("test.lua"), lua)
            .config
            .unwrap();
        assert!(read(&host, "while_loading"));
        run_bind(&host, &mut FakeContext::default());
        assert!(read(&host, "in_callback"));
        host.end_startup();
        run_bind(&host, &mut FakeContext::default());
        assert!(!read(&host, "in_callback"));

        // A reloaded config never is.
        let (_config, host) = while_reloading(|| {
            Config::parse_with_host(std::path::Path::new("test.lua"), lua).config
        })
        .unwrap();
        assert!(!read(&host, "while_loading"));
        run_bind(&host, &mut FakeContext::default());
        assert!(!read(&host, "in_callback"));

        // while_reloading only applies to what runs inside it.
        let (_config, host) = Config::parse_with_host(std::path::Path::new("test.lua"), lua)
            .config
            .unwrap();
        assert!(read(&host, "while_loading"));
    }

    #[test]
    fn keybinding_modes() {
        let config = Config::parse_mem(
            r#"
            neori.mode("resize", function()
                neori.bind("Left", "set-column-width -10%")
                neori.bind("Mod+Q", "exit-mode", { hotkey_overlay_title = "Done" })
            end)
            neori.mode("move", function()
                neori.bind("Left", "move-column-left")   -- same key as in resize: fine
            end)
            neori.bind("Mod+Q", "close-window")          -- same key as in resize: fine
            neori.bind("Mod+R", "enter-mode resize")
            neori.bind("Mod+M", { enter_mode = "move" })
            "#,
        )
        .unwrap();

        // The normal binds don't include the modes' binds.
        let normal: Vec<_> = config
            .binds
            .0
            .iter()
            .map(|b| b.press_action().cloned())
            .collect();
        assert_eq!(
            normal,
            [
                Some(Action::CloseWindow { id: None }),
                Some(Action::EnterMode {
                    name: "resize".into()
                }),
                Some(Action::EnterMode {
                    name: "move".into()
                }),
            ]
        );
        // Modes are sorted by name.
        let names: Vec<_> = config.modes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["move", "resize"]);
        let resize = &config.modes[1].binds.0;
        assert_eq!(resize.len(), 2);
        assert!(matches!(
            resize[1].press_action(),
            Some(Action::ExitMode {})
        ));
        assert_eq!(resize[1].hotkey_overlay_title, Some(Some("Done".into())));
    }

    #[test]
    fn keybinding_mode_errors() {
        let err = |lua: &str| format!("{:?}", Config::parse_mem(lua).unwrap_err());

        let msg = err(r#"neori.bind("Mod+R", "enter-mode resize")"#);
        assert!(
            msg.contains("but no mode has that name") && msg.contains("resize"),
            "{msg}"
        );

        let msg = err(r#"neori.mode("a", function() end) neori.mode("a", function() end)"#);
        assert!(msg.contains("declared twice"), "{msg}");

        let msg = err(r#"neori.mode("a", function() neori.mode("b", function() end) end)"#);
        assert!(msg.contains("can't be nested"), "{msg}");

        let msg = err(
            r#"neori.mode("a", function() neori.bind("Left", "close-window") neori.bind("Left", "close-window") end)"#,
        );
        assert!(msg.contains("duplicate keybind: Left"), "{msg}");

        // An error inside the mode's function still ends the mode: later binds are normal.
        let config = Config::parse_mem(
            r#"
            pcall(neori.mode, "a", function() error("oops") end)
            neori.bind("Mod+T", "close-window")
            "#,
        )
        .unwrap();
        assert_eq!(config.binds.0.len(), 1);
    }

    #[test]
    fn keybinding_mode_functions() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.mode("resize", function() end)
            neori.bind("Mod+T", function()
                neori.enter_mode("resize")
                neori.exit_mode()
                seen = neori.current_mode()
                ok, err = pcall(neori.enter_mode, "nope")
                err = tostring(err)
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext::default();
        let actions = run_bind(&host, &mut ctx);
        assert_eq!(
            actions,
            [
                Action::EnterMode {
                    name: "resize".into()
                },
                Action::ExitMode {}
            ]
        );
        let mlua::Value::String(err) = host.read_global("err").unwrap() else {
            panic!("err is not a string");
        };
        assert!(err.to_str().unwrap().contains("no mode is named \"nope\""));
    }

    #[test]
    fn window_maximize_center_and_kill() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+T", function()
                local w = neori.focused_window()
                w:maximize(false)   -- not maximized: nothing to do
                w:maximize(true)    -- toggles
                w:maximize()        -- always toggles
                w:center()
                pid = w:pid()
                geometry = w:geometry()
                ok, err = pcall(function() w:kill() end)
                err = tostring(err)
            end)
            "#,
        )
        .config
        .unwrap();
        let mut ctx = FakeContext::new(
            vec![WindowSnapshot {
                id: 5,
                pid: Some(4242),
                ..Default::default()
            }],
            Some(5),
        );
        let actions = run_bind(&host, &mut ctx);
        assert_eq!(
            actions,
            [
                Action::MaximizeWindowToEdges { id: Some(5) },
                Action::MaximizeWindowToEdges { id: Some(5) },
                Action::CenterWindow { id: Some(5) },
            ]
        );
        assert!(matches!(
            host.read_global("pid").unwrap(),
            mlua::Value::Integer(4242)
        ));
        // The fake context knows no geometry and can't kill.
        assert!(matches!(
            host.read_global("geometry").unwrap(),
            mlua::Value::Nil
        ));
        let mlua::Value::String(err) = host.read_global("err").unwrap() else {
            panic!("err is not a string");
        };
        assert!(
            err.to_str().unwrap().contains("isn't supported"),
            "{:?}",
            err
        );
    }

    #[test]
    fn timers() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            -- Made while the config loads: running, armed once it has loaded.
            once = neori.after(100, function() end)
            ticker = neori.every(50, function() end)
            stopped = neori.every(10, function() end)
            stopped:stop()
            loading = { tostring(once), tostring(stopped), ticker:timeout() }
            neori.bind("Mod+T", function()
                ticker:stop()
                ticker:stop()        -- already stopped: nothing to do
                ticker:set_timeout(75)
                ticker:start()
                ticker:start()       -- already running: keeps its countdown
                once:again()         -- restarts it
                running = ticker:is_running()
            end)
            ok, err = pcall(neori.every, 0, function() end)
            err = tostring(err)
            "#,
        )
        .config
        .unwrap();
        let mlua::Value::Table(loading) = host.read_global("loading").unwrap() else {
            panic!("loading is not a table");
        };
        assert_eq!(
            loading.get::<String>(1).unwrap(),
            "Timer(after 100 ms, running)"
        );
        assert_eq!(
            loading.get::<String>(2).unwrap(),
            "Timer(every 10 ms, stopped)"
        );
        assert_eq!(loading.get::<i64>(3).unwrap(), 50);
        let mlua::Value::String(err) = host.read_global("err").unwrap() else {
            panic!("err is not a string");
        };
        assert!(err.to_str().unwrap().contains("at least 1 ms"));

        // The compositor arms the running ones after loading the config.
        let running = host.running_timers();
        let ids: Vec<u64> = running.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(running[0].1, std::time::Duration::from_millis(100));
        assert_eq!(running[1].1, std::time::Duration::from_millis(50));
        let (once, ticker) = (ids[0], ids[1]);

        let mut ctx = FakeContext::default();
        run_bind(&host, &mut ctx);
        assert_eq!(
            ctx.timer_calls,
            [(ticker, None), (ticker, Some(75)), (once, Some(100))]
        );
        assert!(matches!(
            host.read_global("running").unwrap(),
            mlua::Value::Boolean(true)
        ));
    }

    #[test]
    fn firing_a_one_shot_timer_stops_it() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.after(5, function() end)
            neori.every(5, function() end)
            "#,
        )
        .config
        .unwrap();
        let ids: Vec<u64> = host.running_timers().iter().map(|(id, _)| *id).collect();
        let mut ctx = FakeContext::default();
        for &id in &ids {
            let call = host.timer_call(id).unwrap();
            with_script_context(&mut ctx, || call.run(LuaTimer(id))).unwrap();
        }
        // The one-shot is done; the repeating one fires again.
        assert_eq!(host.timer_rearm(ids[0]), None);
        assert_eq!(
            host.timer_rearm(ids[1]),
            Some(std::time::Duration::from_millis(5))
        );
        assert!(host.timer_call(ids[0]).is_none());
    }

    #[test]
    fn bind_options_argument() {
        // neori.bind(key, action, options) works for every kind of action.
        let config = Config::parse_mem(
            r#"
            local spawn = neori.spawn("wl-freeze")
            neori.bind("Mod+A", function() end, {
                ["repeat"] = false,
                cooldown_ms = 150,
                allow_when_locked = true,
                allow_inhibiting = false,
                hotkey_overlay_title = "Function",
            })
            neori.bind("Mod+B", spawn, { hotkey_overlay_title = "Pause program", ["repeat"] = false })
            -- The same action table in another bind doesn't pick up the options above.
            neori.bind("Mod+C", spawn)
            neori.bind("Mod+D", "close-window", { hotkey_overlay_title = "Close" })
            "#,
        )
        .unwrap();
        let binds = &config.binds.0;

        assert!(matches!(
            binds[0].press_action(),
            Some(Action::ScriptFunction { .. })
        ));
        assert!(!binds[0].repeat);
        assert_eq!(
            binds[0].cooldown,
            Some(std::time::Duration::from_millis(150))
        );
        assert!(binds[0].allow_when_locked);
        assert!(!binds[0].allow_inhibiting);
        assert_eq!(binds[0].hotkey_overlay_title, Some(Some("Function".into())));

        assert!(matches!(
            binds[1].press_action(),
            Some(Action::Spawn { .. })
        ));
        assert!(!binds[1].repeat);
        assert_eq!(
            binds[1].hotkey_overlay_title,
            Some(Some("Pause program".into()))
        );

        assert!(binds[2].repeat);
        assert_eq!(binds[2].hotkey_overlay_title, None);

        assert!(matches!(
            binds[3].press_action(),
            Some(Action::CloseWindow { .. })
        ));
        assert_eq!(binds[3].hotkey_overlay_title, Some(Some("Close".into())));
    }

    #[test]
    fn bind_options_errors() {
        let err = |lua: &str| format!("{:?}", Config::parse_mem(lua).unwrap_err());

        let msg = err(r#"neori.bind("Mod+A", "close-window", { reapeat = false })"#);
        assert!(msg.contains("unknown bind option `reapeat`"), "{msg}");

        let msg = err(
            r#"neori.bind("Mod+A", { close_window = true, hotkey_overlay_title = "A" },
                { hotkey_overlay_title = "B" })"#,
        );
        assert!(
            msg.contains("`hotkey_overlay_title` is set both in the action and in the options"),
            "{msg}"
        );
    }

    #[test]
    fn function_binds_take_options_in_table_form() {
        // { press = function, <options> } gives a Lua function bind the usual bind options.
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+T", {
                press = function() end,
                ["repeat"] = false,
                cooldown_ms = 150,
                allow_when_locked = true,
                allow_inhibiting = false,
                hotkey_overlay_title = "Do the thing",
            })
            "#,
        )
        .unwrap();
        let bind = &config.binds.0[0];
        assert!(matches!(
            bind.press_action(),
            Some(Action::ScriptFunction { .. })
        ));
        assert!(!bind.repeat);
        assert_eq!(bind.cooldown, Some(std::time::Duration::from_millis(150)));
        assert!(bind.allow_when_locked);
        assert!(!bind.allow_inhibiting);
        assert_eq!(
            bind.hotkey_overlay_title,
            Some(Some("Do the thing".to_string()))
        );
    }

    #[test]
    fn neori_windows_reads_live_windows() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            seen = 0
            neori.on("startup", function()
                for _, w in ipairs(neori.windows()) do
                    seen = seen + 1
                end
                focused_app_id = neori.focused_window() and neori.focused_window():app_id() or nil
            end)
            "#,
        )
        .config
        .unwrap();

        let w1 = WindowSnapshot {
            id: 1,
            app_id: Some("a".into()),
            is_focused: false,
            ..Default::default()
        };
        let w2 = WindowSnapshot {
            id: 2,
            app_id: Some("b".into()),
            is_focused: true,
            ..Default::default()
        };
        let mut ctx = FakeContext::new(vec![w1, w2], Some(2));
        with_script_context(&mut ctx, || host.call_event("startup")).unwrap();

        let seen = host.read_global("seen").unwrap();
        assert!(matches!(seen, mlua::Value::Integer(2)));
        let app = host.read_global("focused_app_id").unwrap();
        let mlua::Value::String(s) = app else {
            panic!("focused_app_id is not a string");
        };
        assert_eq!(s.to_str().unwrap().to_string(), "b");
    }

    #[test]
    fn lua_function_bind_parses_as_script_function() {
        let (config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function() end)
            neori.bind("Mod+Alt+K", "close-window")
            "#,
        )
        .config
        .unwrap();

        assert_eq!(config.binds.0.len(), 2);

        // The function bind carries a ScriptFunction action registered with the host.
        let Some(&Action::ScriptFunction { id }) = config.binds.0[0].press_action() else {
            panic!(
                "expected a function bind, got {:?}",
                config.binds.0[0].action
            );
        };
        assert!(
            host.has_bind_function(id),
            "ScriptHost should have the function registered for id {id}"
        );

        // The other bind carries the static Action.
        assert_eq!(
            config.binds.0[1].action,
            BoundAction::Press(Action::CloseWindow { id: None })
        );
    }

    #[test]
    fn neori_action_runs_actions() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                neori.action("spawn", { "kitty" })
                neori.action("close-window")
                neori.action("focus-column-left")
            end)
            "#,
        )
        .config
        .unwrap();

        // Trigger the Lua function; neori.action carried out three actions.
        // The first bind function registered in this host gets id 1.
        let actions = run_bind(&host, &mut FakeContext::default());
        assert_eq!(actions.len(), 3, "got {actions:?}");
        assert!(matches!(actions[0], Action::Spawn { .. }));
        assert!(matches!(actions[1], Action::CloseWindow { id: None }));
        assert!(matches!(actions[2], Action::FocusColumnLeft {}));
    }

    #[test]
    fn lua_function_bind_error_is_surfaced() {
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                error("intentional")
            end)
            "#,
        )
        .config
        .unwrap();

        let err = host.call_bind_function(1).expect_err("expected error");
        assert!(err.contains("intentional"), "got {err:?}");
    }

    #[test]
    fn create_virtual_output_action_parses() {
        // Full form with all options.
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Shift+V", neori.create_virtual_output {
                name = "sunshine",
                width = 1920,
                height = 1080,
                refresh_rate = 144,
            })
            "#,
        )
        .unwrap();
        assert!(actions(&config).contains(&Action::CreateVirtualOutput {
            name: Some("sunshine".to_string()),
            width: Some(1920),
            height: Some(1080),
            refresh_rate: Some(144),
        }));

        // No-arg form uses all defaults.
        let config =
            Config::parse_mem(r#"neori.bind("Mod+Shift+V", neori.create_virtual_output())"#)
                .unwrap();
        assert!(actions(&config).contains(&Action::CreateVirtualOutput {
            name: None,
            width: None,
            height: None,
            refresh_rate: None,
        }));

        // kebab-case alias accepted.
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Shift+V", neori.create_virtual_output {
                name = "virt",
                ["refresh-rate"] = 60,
            })
            "#,
        )
        .unwrap();
        assert!(actions(&config).contains(&Action::CreateVirtualOutput {
            name: Some("virt".to_string()),
            width: None,
            height: None,
            refresh_rate: Some(60),
        }));

        // String action shorthand.
        let config =
            Config::parse_mem(r#"neori.bind("Mod+Shift+V", "create-virtual-output")"#).unwrap();
        assert!(actions(&config).contains(&Action::CreateVirtualOutput {
            name: None,
            width: None,
            height: None,
            refresh_rate: None,
        }));
    }

    #[test]
    fn remove_virtual_output_action_parses() {
        // String arg form.
        let config =
            Config::parse_mem(r#"neori.bind("Mod+Shift+R", neori.remove_virtual_output("virt"))"#)
                .unwrap();
        assert!(actions(&config).contains(&Action::RemoveVirtualOutput {
            name: "virt".to_string(),
        }));

        // Table form with trailing bind metadata.
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Shift+R", neori.remove_virtual_output {
                name = "sunshine",
                ["repeat"] = false,
            })
            "#,
        )
        .unwrap();
        assert!(actions(&config).contains(&Action::RemoveVirtualOutput {
            name: "sunshine".to_string(),
        }));

        // String action form ("remove-virtual-output <name>").
        let config =
            Config::parse_mem(r#"neori.bind("Mod+Shift+R", "remove-virtual-output virt")"#)
                .unwrap();
        assert!(actions(&config).contains(&Action::RemoveVirtualOutput {
            name: "virt".to_string(),
        }));
    }

    #[test]
    fn scratchpad_actions_parse() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Alt+minus", "send-to-scratchpad")
            neori.bind("Mod+Alt+Return", "toggle-scratchpad")
            neori.bind("Mod+Alt+plus", { remove_from_scratchpad = true })
            "#,
        )
        .unwrap();

        let actions = actions(&config);
        assert!(actions.contains(&Action::SendToScratchpad { name: None }));
        assert!(actions.contains(&Action::ToggleScratchpad { name: None }));
        assert!(actions.contains(&Action::RemoveFromScratchpad { name: None }));
    }

    #[test]
    fn named_scratchpad_actions_parse() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Alt+m", "send-to-scratchpad term")
            neori.bind("Mod+Alt+Return", "toggle-scratchpad chat")
            neori.bind("Mod+Alt+minus", { send_to_scratchpad = "notes" })
            neori.bind("Mod+Alt+plus", { remove_from_scratchpad = "notes" })
            neori.bind("Mod+Alt+n", { toggle_scratchpad = { name = "notes" } })
            "#,
        )
        .unwrap();

        let actions = actions(&config);
        assert!(actions.contains(&Action::SendToScratchpad {
            name: Some("term".to_string()),
        }));
        assert!(actions.contains(&Action::ToggleScratchpad {
            name: Some("chat".to_string()),
        }));
        assert!(actions.contains(&Action::SendToScratchpad {
            name: Some("notes".to_string()),
        }));
        assert!(actions.contains(&Action::RemoveFromScratchpad {
            name: Some("notes".to_string()),
        }));
        assert!(actions.contains(&Action::ToggleScratchpad {
            name: Some("notes".to_string()),
        }));
    }

    #[test]
    fn scratchpads_parse() {
        let config = Config::parse_mem(
            r#"
            neori.scratchpad { spawn = { "kitty" } }
            neori.scratchpad("term", { spawn_sh = "alacritty -e htop" })
            "#,
        )
        .unwrap();

        assert_eq!(
            config.scratchpads,
            vec![
                Scratchpad {
                    name: String::new(),
                    spawn: Some(vec!["kitty".to_string()]),
                    spawn_sh: None,
                },
                Scratchpad {
                    name: "term".to_string(),
                    spawn: None,
                    spawn_sh: Some("alacritty -e htop".to_string()),
                },
            ]
        );

        // Top-level section form works too.
        let config = Config::parse_mem(
            r#"
            return {
                scratchpads = {
                    { name = "browser", spawn = { "firefox" } },
                },
            }
            "#,
        )
        .unwrap();
        assert_eq!(config.scratchpads.len(), 1);
        assert_eq!(config.scratchpads[0].name, "browser");
        assert_eq!(
            config.scratchpads[0].spawn,
            Some(vec!["firefox".to_string()])
        );

        // Duplicate names are a configuration error.
        assert!(Config::parse_mem(
            r#"
            neori.scratchpad("term", { spawn = { "kitty" } })
            neori.scratchpad("term", { spawn = { "foot" } })
            "#,
        )
        .is_err());

        // So is setting both spawn forms at once.
        assert!(Config::parse_mem(
            r#"
            neori.scratchpad("term", { spawn = { "kitty" }, spawn_sh = "kitty" })
            "#,
        )
        .is_err());
    }

    #[test]
    fn require_includes_merge_into_config() {
        let dir = std::env::temp_dir().join(format!("neori-require-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config_path = dir.join("config.lua");
        let binds_path = dir.join("binds.lua");
        let nested_path = dir.join("nested.lua");

        std::fs::write(&binds_path, "neori.bind(\"Mod+Return\", \"spawn kitty\")").unwrap();
        std::fs::write(&nested_path, "-- just a comment").unwrap();
        std::fs::write(
            &config_path,
            "require \"binds\"\nrequire \"nested\"\nreturn { input = { keyboard = { numlock = true } } }\n",
        )
        .unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        let result = Config::parse(&config_path, &content);

        let config = result.config.expect("config should parse");
        assert_eq!(
            result.includes,
            vec![config_path, binds_path.clone(), nested_path]
        );
        assert!(config.input.keyboard.numlock);
        let actions = actions(&config);
        assert!(actions.contains(&Action::Spawn {
            command: vec!["kitty".to_string()]
        }));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn broken_require_still_reports_includes() {
        let dir = std::env::temp_dir().join(format!("neori-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config_path = dir.join("config.lua");
        let broken_path = dir.join("broken.lua");
        std::fs::write(&broken_path, "not valid lua (").unwrap();
        std::fs::write(&config_path, "require \"broken\"").unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        let result = Config::parse(&config_path, &content);

        assert!(result.config.is_err(), "config should fail to parse");
        assert_eq!(result.includes, vec![broken_path]);

        let _ = std::fs::remove_dir_all(dir);
    }

    fn diff_lines(expected: &str, actual: &str) -> String {
        let mut output = String::new();
        let mut in_change = false;

        for change in diff::lines(expected, actual) {
            match change {
                diff::Result::Both(_, _) => {
                    in_change = false;
                }
                diff::Result::Left(line) => {
                    if !output.is_empty() && !in_change {
                        output.push('\n');
                    }
                    output.push('-');
                    output.push_str(line);
                    output.push('\n');
                    in_change = true;
                }
                diff::Result::Right(line) => {
                    if !output.is_empty() && !in_change {
                        output.push('\n');
                    }
                    output.push('+');
                    output.push_str(line);
                    output.push('\n');
                    in_change = true;
                }
            }
        }

        output
    }

    #[test]
    fn diff_empty_to_default() {
        // We try to write the config defaults in such a way that empty sections (and an empty
        // config) give the same outcome as the default config bundled with neori. This test
        // verifies the actual differences between the two.
        let mut default_config = Config::load_default();
        let empty_config = Config::parse_mem("").unwrap();

        // Some notable omissions: the default config has some window rules, and an empty config
        // will not have any binds. Clear them out so they don't spam the diff.
        default_config.window_rules.clear();
        default_config.binds.0.clear();

        assert_snapshot!(
            diff_lines(
                &format!("{empty_config:#?}"),
                &format!("{default_config:#?}")
            ),
            @r#"
        -            numlock: false,
        +            numlock: true,

        -            tap: false,
        +            tap: true,

        -            natural_scroll: false,
        +            natural_scroll: true,

        -    startup: [],
        +    startup: [
        +        Spawn {
        +            command: [
        +                "waybar",
        +            ],
        +        },
        +    ],
        "#,
        );
    }

    fn actions(config: &Config) -> Vec<Action> {
        config
            .binds
            .0
            .iter()
            .filter_map(|b| b.press_action().cloned())
            .collect()
    }

    #[test]
    fn modifier_binds_keep_their_trigger() {
        use crate::binds::{Modifiers, Trigger};
        use crate::input::ModKey;

        let config = Config::parse_mem(
            r#"
            neori.bind("Mod", "close-window")
            neori.bind("Ctrl+Alt", "close-window")
            neori.bind("Shift+Alt_L", "close-window")
            neori.bind("Super+Q", "close-window")
            "#,
        )
        .unwrap();
        let triggers: Vec<_> = config.binds.0.iter().map(|b| b.key.trigger).collect();

        // Bare modifiers and modifier keysyms aren't converted to keycodes, even though
        // keycode binds are the default; other keys still are.
        assert_eq!(triggers[0], Trigger::CompositorMod);
        assert_eq!(triggers[1], Trigger::Modifier(ModKey::Alt));
        assert_eq!(
            triggers[2],
            Trigger::Keysym(smithay::input::keyboard::Keysym::Alt_L)
        );
        assert_eq!(triggers[3], Trigger::Keycode(16));

        // The trigger's own modifier is implied.
        assert_eq!(
            config.binds.0[2].key.modifiers,
            Modifiers::SHIFT | Modifiers::ALT
        );

        // An explicit keycode request on a modifier keysym also keeps the keysym.
        let config =
            Config::parse_mem(r#"neori.bind("Alt_L", { close_window = true, keycode = true })"#)
                .unwrap();
        assert_eq!(
            config.binds.0[0].key.trigger,
            Trigger::Keysym(smithay::input::keyboard::Keysym::Alt_L)
        );
    }

    #[test]
    fn conflicting_modifier_binds_are_rejected() {
        // `Alt+Alt_L` normalizes to the same key as `Alt_L`.
        let err = Config::parse_mem(
            r#"
            neori.bind("Alt_L", "close-window")
            neori.bind("Alt+Alt_L", "close-window")
            "#,
        )
        .unwrap_err();
        assert!(err.main.contains("duplicate keybind: Alt+Alt_L"), "{err}");

        // `Alt` and `Alt_R` are different keys triggered by the same key press.
        let err = Config::parse_mem(
            r#"
            neori.bind("Alt", "close-window")
            neori.bind("Alt_R", "close-window")
            "#,
        )
        .unwrap_err();
        assert!(err.main.contains("conflicting keybinds"), "{err}");

        // Different held modifiers don't conflict.
        Config::parse_mem(
            r#"
            neori.bind("Alt", "close-window")
            neori.bind("Ctrl+Alt", "close-window")
            "#,
        )
        .unwrap();
    }

    #[test]
    fn press_and_release_binds() {
        let (config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod", { release = "toggle-overview" })
            neori.bind("Mod+Q", { press = "close-window", release = neori.spawn("foot") })
            neori.bind("Mod+F", { press = function() end, release = function() end })
            neori.bind("Mod+T", { press = { toggle_overview = true } })
            neori.bind("Mod+E", { release = "toggle-keyboard-shortcuts-inhibit" })
            "#,
        )
        .config
        .unwrap();
        let binds = &config.binds.0;

        assert_eq!(
            binds[0].action,
            BoundAction::Release(Action::ToggleOverview {})
        );
        // Binds with a release action don't repeat by default.
        assert!(!binds[0].repeat);

        assert_eq!(
            binds[1].action,
            BoundAction::Both {
                press: Action::CloseWindow { id: None },
                release: Action::Spawn {
                    command: vec!["foot".to_string()]
                },
            }
        );

        // Lua functions work in both slots, each registered separately.
        let BoundAction::Both {
            press: Action::ScriptFunction { id: press },
            release: Action::ScriptFunction { id: release },
        } = binds[2].action
        else {
            panic!("expected two function actions, got {:?}", binds[2].action);
        };
        assert_ne!(press, release);
        assert!(host.has_bind_function(press));
        assert!(host.has_bind_function(release));

        // A press-only slot is the same as a direct action, and still repeats.
        assert_eq!(
            binds[3].action,
            BoundAction::Press(Action::ToggleOverview {})
        );
        assert!(binds[3].repeat);

        // The inhibit toggle is never inhibitable, even as a release action.
        assert!(!binds[4].allow_inhibiting);
    }

    #[test]
    fn layer_rule_focus_on_type_in_overview() {
        let config = Config::parse_mem(
            r#"
            neori.layer_rule {
                match = { namespace = "^launcher$" },
                focus_on_type_in_overview = true,
            }
            neori.layer_rule { ["focus-on-type-in-overview"] = false }
            neori.layer_rule { block_out_from = "screencast" }
            "#,
        )
        .unwrap();
        let rules = &config.layer_rules;
        assert_eq!(rules[0].focus_on_type_in_overview, Some(true));
        assert_eq!(rules[1].focus_on_type_in_overview, Some(false));
        assert_eq!(rules[2].focus_on_type_in_overview, None);
    }

    #[test]
    fn block_out_from() {
        let config = Config::parse_mem(
            r#"
            neori.window_rule { block_out_from = "screencast" }
            neori.window_rule { block_out_from = { "screen-capture", kind = "hide" } }
            neori.window_rule { ["block-out-from"] = { target = "screencast", kind = "fill" } }
            neori.window_rule { opacity = 0.5 }
            neori.layer_rule { block_out_from = { "screencast", kind = "hide" } }
            "#,
        )
        .unwrap();

        let rule = |target, kind| Some(BlockOutFrom { target, kind });
        let rules = &config.window_rules;
        assert_eq!(
            rules[0].block_out_from,
            rule(BlockOutTarget::Screencast, BlockOutKind::Fill)
        );
        assert_eq!(
            rules[1].block_out_from,
            rule(BlockOutTarget::ScreenCapture, BlockOutKind::Hide)
        );
        assert_eq!(
            rules[2].block_out_from,
            rule(BlockOutTarget::Screencast, BlockOutKind::Fill)
        );
        assert_eq!(rules[3].block_out_from, None);
        assert_eq!(
            config.layer_rules[0].block_out_from,
            rule(BlockOutTarget::Screencast, BlockOutKind::Hide)
        );

        // Invalid values are rejected in strict mode.
        for text in [
            r#"neori.window_rule { block_out_from = "everything" }"#,
            r#"neori.window_rule { block_out_from = { "screencast", kind = "blur" } }"#,
        ] {
            assert!(
                Config::parse_mem(&format!("neori.strict()\n{text}")).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn pass_through_binds() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Alt", { release = "toggle-overview", pass_through = true })
            neori.bind("Mod+Q", { close_window = true, ["pass-through"] = false })
            neori.bind("Mod+T", "close-window")
            "#,
        )
        .unwrap();
        let binds = &config.binds.0;
        assert_eq!(binds[0].pass_through, Some(true));
        assert_eq!(binds[1].pass_through, Some(false));
        assert_eq!(binds[2].pass_through, None);

        // Only keyboard binds can pass through.
        let err = Config::parse_mem(
            r#"neori.bind("Mod+MouseLeft", { close_window = true, pass_through = true })"#,
        )
        .unwrap_err();
        assert!(
            err.main.contains("only supported for keyboard binds"),
            "{err}"
        );
    }

    #[test]
    fn press_and_release_bind_errors() {
        let err = |text: &str| Config::parse_mem(text).unwrap_err().main;

        assert!(err(
            r#"neori.bind("Mod+Q", { close_window = true, release = "toggle-overview" })"#
        )
        .contains("cannot mix a direct action with press/release actions"));
        assert!(
            err(r#"neori.bind("Mod+WheelScrollDown", { release = "focus-workspace-down" })"#)
                .contains("release actions are not supported for scroll binds")
        );
        assert!(
            err(r#"neori.bind("Mod", { release = "toggle-overview", ["repeat"] = true })"#)
                .contains("release-only binds cannot repeat")
        );
        assert!(err(r#"neori.bind("Mod", { release = "no-such-action" })"#).contains("release"));
        assert!(
            err(r#"neori.bind("Mod", { release = { allow_when_locked = true } })"#)
                .contains("bind options go on the bind itself")
        );
    }

    #[test]
    fn keycode_is_the_default() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+Slash", neori.spawn("rofi", "-show", "run", { hotkey_overlay_title = "Binary Launcher" }))
            neori.bind("Mod+Shift+Slash", { show_hotkey_overlay = true })
            "#,
        )
        .unwrap();

        let binds = &config.binds.0;
        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0].key.trigger, Trigger::Keycode(53));
        assert_eq!(binds[1].key.trigger, Trigger::Keycode(53));
    }

    #[test]
    fn keycode_false_keeps_the_keysym() {
        let config = Config::parse_mem(
            r#"neori.bind("Mod+Slash", { show_hotkey_overlay = true, keycode = false })"#,
        )
        .unwrap();

        assert_eq!(
            config.binds.0[0].key.trigger,
            Trigger::Keysym(Keysym::slash)
        );
    }

    #[test]
    fn keycode_falls_back_to_keysym_for_unknown_keys() {
        // A keysym with no US-layout keycode (e.g. a layout-specific key) keeps the keysym
        // by default instead of erroring.
        let config = Config::parse_mem(r#"neori.bind("XF86Keyboard", "spawn true")"#).unwrap();

        assert_eq!(
            config.binds.0[0].key.trigger,
            Trigger::Keysym(Keysym::XF86_Keyboard)
        );
    }

    #[test]
    fn neori_api_sections() {
        let config = Config::parse_mem(
            r#"
            neori.input {
                touchpad = { tap = true },
            }
            neori.layout { gaps = 8 }
            neori.screenshot_path("/tmp/shot.png")
            neori.on("startup", "spawn waybar")
            neori.bind("Mod+Return", "spawn kitty")
            neori.bind("Mod+1", { focus_workspace = 1 })
            "#,
        )
        .unwrap();

        assert!(config.input.touchpad.tap);
        assert_eq!(config.layout.gaps, 8.0);
        assert_eq!(config.screenshot_path.0.as_deref(), Some("/tmp/shot.png"));
        assert_eq!(
            config.startup,
            [Action::Spawn {
                command: vec!["waybar".to_string()]
            }]
        );
        assert_eq!(config.binds.0.len(), 2);
        let actions = actions(&config);
        assert!(actions.contains(&Action::Spawn {
            command: vec!["kitty".to_string()]
        }));
        assert!(actions.contains(&Action::FocusWorkspace {
            reference: WorkspaceReference::Index(1)
        }));
    }

    #[test]
    fn neori_api_monitor_events() {
        let config = Config::parse_mem(
            r#"
            neori.on("monitor-added", neori.spawn("waybar"))
            neori.on("monitor_added", "spawn waybar2")
            neori.on("monitor_removed", "spawn cleanup")
            neori.on("monitor-removed", neori.spawn_sh("echo bye"))
            "#,
        )
        .unwrap();

        assert_eq!(
            config.monitor_added,
            [
                Action::Spawn {
                    command: vec!["waybar".to_string()]
                },
                Action::Spawn {
                    command: vec!["waybar2".to_string()]
                },
            ]
        );
        assert_eq!(
            config.monitor_removed,
            [
                Action::Spawn {
                    command: vec!["cleanup".to_string()]
                },
                Action::SpawnSh {
                    command: "echo bye".to_string()
                },
            ]
        );

        let err = Config::parse_mem(r#"neori.on("bogus", "spawn waybar")"#).unwrap_err();
        assert!(err.main.contains("unknown event 'bogus'"), "{err}");
    }

    #[test]
    fn neori_api_spawn_builder() {
        let config = Config::parse_mem(
            r#"
            neori.on("startup", neori.spawn("waybar"))
            neori.bind("Mod+Return", neori.spawn("kitty"))
            neori.bind("Mod+T", neori.spawn { "kitty", "-e", "sh" })
            neori.bind("Mod+B", neori.spawn("brightnessctl", "set", "+10%", { allow_when_locked = true }))
            neori.bind("Mod+C", neori.spawn_sh("wl-mirror $(echo hi)"))
            neori.bind("Mod+M", neori.spawn("terminal-meta", { hotkey_overlay_title = "Terminal", ["repeat"] = false }))
            "#,
        )
        .unwrap();

        assert_eq!(
            config.startup,
            [Action::Spawn {
                command: vec!["waybar".to_string()]
            }]
        );
        let actions = actions(&config);
        assert!(actions.contains(&Action::Spawn {
            command: vec!["kitty".to_string()]
        }));
        assert!(actions.contains(&Action::Spawn {
            command: vec!["kitty".to_string(), "-e".to_string(), "sh".to_string()]
        }));
        assert!(actions.contains(&Action::Spawn {
            command: vec![
                "brightnessctl".to_string(),
                "set".to_string(),
                "+10%".to_string()
            ]
        }));
        assert!(actions.contains(&Action::SpawnSh {
            command: "wl-mirror $(echo hi)".to_string()
        }));

        let brightness = config
            .binds
            .0
            .iter()
            .find(|b| {
                b.press_action()
                    == Some(&Action::Spawn {
                        command: vec![
                            "brightnessctl".to_string(),
                            "set".to_string(),
                            "+10%".to_string(),
                        ],
                    })
            })
            .unwrap();
        assert!(brightness.allow_when_locked);

        let meta = config
            .binds
            .0
            .iter()
            .find(|b| {
                b.press_action()
                    == Some(&Action::Spawn {
                        command: vec!["terminal-meta".to_string()],
                    })
            })
            .unwrap();
        assert_eq!(
            meta.hotkey_overlay_title,
            Some(Some("Terminal".to_string()))
        );
        assert!(!meta.repeat);

        let err = Config::parse_mem("neori.spawn()").unwrap_err();
        assert!(err.main.contains("spawn requires a command"), "{err}");
    }

    #[test]
    fn neori_api_extended_actions() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+A", "focus_column_first")
            neori.bind("Mod+B", "move_column_to_monitor_left")
            neori.bind("Mod+C", "move_workspace_down")
            neori.bind("Mod+D", "consume_or_expel_window_right")
            neori.bind("Mod+E", "switch_preset_window_height")
            neori.bind("Mod+F", "reset_window_height")
            neori.bind("Mod+G", "expand_column_to_available_width")
            neori.bind("Mod+H", "center_visible_columns")
            neori.bind("Mod+I", "switch_focus_between_floating_and_tiling")
            neori.bind("Mod+J", "move_column_to_workspace_down")
            neori.bind("Mod+K", { set_column_width = "-10%" })
            neori.bind("Mod+L", { set_window_height = "+10%" })
            "#,
        )
        .unwrap();

        let actions = actions(&config);
        assert!(actions.contains(&Action::FocusColumnFirst {}));
        assert!(actions.contains(&Action::MoveColumnToMonitorLeft {}));
        assert!(actions.contains(&Action::MoveWorkspaceDown {}));
        assert!(actions.contains(&Action::ConsumeOrExpelWindowRight { id: None }));
        assert!(actions.contains(&Action::SwitchPresetWindowHeight { id: None }));
        assert!(actions.contains(&Action::ResetWindowHeight { id: None }));
        assert!(actions.contains(&Action::ExpandColumnToAvailableWidth {}));
        assert!(actions.contains(&Action::CenterVisibleColumns {}));
        assert!(actions.contains(&Action::SwitchFocusBetweenFloatingAndTiling {}));
        assert!(actions.contains(&Action::MoveColumnToWorkspaceDown { focus: true }));
        assert!(actions.contains(&Action::SetColumnWidth {
            change: neori_ipc::SizeChange::AdjustProportion(-10.0)
        }));
        assert!(actions.contains(&Action::SetWindowHeight {
            id: None,
            change: neori_ipc::SizeChange::AdjustProportion(10.0)
        }));
    }

    #[test]
    fn debug_disable_pipewire_dmabuf() {
        let config = Config::parse_mem("neori.debug { disable_pipewire_dmabuf = true }").unwrap();
        assert!(config.debug.disable_pipewire_dmabuf);

        let config =
            Config::parse_mem(r#"neori.debug { ["disable-pipewire-dmabuf"] = true }"#).unwrap();
        assert!(config.debug.disable_pipewire_dmabuf);

        let config = Config::parse_mem("").unwrap();
        assert!(!config.debug.disable_pipewire_dmabuf);
    }

    #[test]
    fn debug_nested_show_host_cursor() {
        let config = Config::parse_mem("neori.debug { nested_show_host_cursor = true }").unwrap();
        assert!(config.debug.nested_show_host_cursor);

        let config =
            Config::parse_mem(r#"neori.debug { ["nested-show-host-cursor"] = true }"#).unwrap();
        assert!(config.debug.nested_show_host_cursor);

        let config = Config::parse_mem("").unwrap();
        assert!(!config.debug.nested_show_host_cursor);
    }

    #[test]
    fn debug_disable_10bit_output() {
        let config = Config::parse_mem("neori.debug { disable_10bit_output = true }").unwrap();
        assert!(config.debug.disable_10bit_output);

        let config =
            Config::parse_mem(r#"neori.debug { ["disable-10bit-output"] = true }"#).unwrap();
        assert!(config.debug.disable_10bit_output);

        let config = Config::parse_mem("neori.debug { disable_10bit_output = false }").unwrap();
        assert!(!config.debug.disable_10bit_output);

        let config = Config::parse_mem("").unwrap();
        assert!(!config.debug.disable_10bit_output);
    }

    #[test]
    fn change_vt_action() {
        let config = Config::parse_mem(r#"neori.bind("Mod+F2", { change_vt = 2 })"#).unwrap();
        assert_eq!(actions(&config), [Action::ChangeVt { vt: 2 }]);
    }

    #[test]
    fn set_and_unset_fullscreen_and_maximized_actions() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+1", "set-window-fullscreen")
            neori.bind("Mod+2", { unset_window_fullscreen = true })
            neori.bind("Mod+3", "set-windowed-fullscreen")
            neori.bind("Mod+4", "unset-windowed-fullscreen")
            neori.bind("Mod+5", { set_window_maximized_to_edges = true })
            neori.bind("Mod+6", "unset-window-maximized-to-edges")
            "#,
        )
        .unwrap();
        assert_eq!(
            actions(&config),
            [
                Action::SetWindowFullscreen { id: None },
                Action::UnsetWindowFullscreen { id: None },
                Action::SetWindowedFullscreen { id: None },
                Action::UnsetWindowedFullscreen { id: None },
                Action::SetWindowMaximizedToEdges { id: None },
                Action::UnsetWindowMaximizedToEdges { id: None },
            ]
        );
    }

    #[test]
    fn toggle_touchpad_action() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+F9", "toggle-touchpad")
            neori.bind("Mod+F10", { toggle_touchpad = true })
            "#,
        )
        .unwrap();

        let actions = actions(&config);
        assert_eq!(
            actions,
            [Action::ToggleTouchpad {}, Action::ToggleTouchpad {}]
        );
    }

    #[test]
    fn neori_api_screenshot_and_debug_actions() {
        let config = Config::parse_mem(
            r#"
            neori.bind("Mod+A", { do_screen_transition = true, delay_ms = 100 })
            neori.bind("Mod+B", "toggle-window-rule-opacity")
            neori.bind("Mod+C", "toggle-debug-tint")
            neori.bind("Mod+D", "debug-toggle-damage")
            neori.bind("Mod+E", "debug-toggle-opaque-regions")
            neori.bind("Mod+G", { quit = true, skip_confirmation = true })
            neori.bind("Mod+H", { screenshot = true, show_pointer = false })
            neori.bind("Mod+I", { screenshot_screen = true, write_to_disk = false, show_pointer = false })
            neori.bind("Mod+J", { screenshot_window = true, write_to_disk = false, show_pointer = true })
            neori.bind("Mod+K", "do-screen-transition")
            "#,
        )
        .unwrap();

        let actions = actions(&config);
        assert!(actions.contains(&Action::DoScreenTransition {
            delay_ms: Some(100)
        }));
        assert!(actions.contains(&Action::ToggleWindowRuleOpacity { id: None }));
        assert!(actions.contains(&Action::ToggleDebugTint {}));
        assert!(actions.contains(&Action::DebugToggleDamage {}));
        assert!(actions.contains(&Action::DebugToggleOpaqueRegions {}));
        assert!(actions.contains(&Action::Quit {
            skip_confirmation: true
        }));
        assert!(actions.contains(&Action::Screenshot {
            show_pointer: false,
            path: None
        }));
        assert!(actions.contains(&Action::ScreenshotScreen {
            write_to_disk: false,
            show_pointer: false,
            path: None
        }));
        assert!(actions.contains(&Action::ScreenshotWindow {
            id: None,
            write_to_disk: false,
            show_pointer: true,
            path: None
        }));
        assert!(actions.contains(&Action::DoScreenTransition { delay_ms: None }));
    }

    #[test]
    fn neori_api_cubic_bezier_curve() {
        let config = Config::parse_mem(
            r#"
            neori.animations {
                window_open = {
                    duration_ms = 250,
                    curve = { "cubic-bezier", 0.05, 0.7, 0.1, 1 },
                },
            }
            "#,
        )
        .unwrap();

        assert_eq!(
            config.animations.window_open.anim.kind,
            crate::animations::Kind::Easing(crate::animations::EasingParams {
                duration_ms: 250,
                curve: crate::animations::Curve::CubicBezier(0.05, 0.7, 0.1, 1.0),
            })
        );
    }

    #[test]
    fn neori_api_pushers_and_set() {
        let config = Config::parse_mem(
            r#"
            neori.output { name = "eDP-1", scale = 1.5 }
            neori.workspace { name = "web", open_on_output = "eDP-1" }
            neori.layer_rule {
                match = { namespace = "waybar" },
                block_out_from = "screen-capture",
            }
            neori.on("startup", { spawn_sh = "qs -c shell" })
            neori.set("prefer_no_csd", true)
            "#,
        )
        .unwrap();

        assert_eq!(config.outputs.0.len(), 1);
        assert_eq!(config.outputs.0[0].name, "eDP-1");
        assert_eq!(config.outputs.0[0].scale, Some(FloatOrInt(1.5)));
        assert_eq!(config.workspaces.len(), 1);
        assert_eq!(config.workspaces[0].name.0, "web");
        assert_eq!(config.layer_rules.len(), 1);
        assert_eq!(config.layer_rules[0].matches.len(), 1);
        assert_eq!(
            config.layer_rules[0].block_out_from,
            Some(BlockOutFrom {
                target: BlockOutTarget::ScreenCapture,
                kind: BlockOutKind::Fill,
            })
        );
        assert_eq!(
            config.startup,
            [Action::SpawnSh {
                command: "qs -c shell".to_string()
            }]
        );
        assert!(config.prefer_no_csd);
    }

    #[test]
    fn workspace_open_on_output_fallbacks() {
        let config = Config::parse_mem(
            r#"
            neori.workspace("chat", { open_on_output = { "DP-1", "HDMI-A-1", "eDP-1" } })
            neori.workspace("web", { ["open-on-output"] = "eDP-1" })
            neori.workspace("misc", {})
            "#,
        )
        .unwrap();

        assert_eq!(
            config.workspaces[0].open_on_output,
            ["DP-1", "HDMI-A-1", "eDP-1"]
        );
        assert_eq!(config.workspaces[1].open_on_output, ["eDP-1"]);
        assert!(config.workspaces[2].open_on_output.is_empty());

        let err = Config::parse_mem(r#"neori.workspace("x", { open_on_output = 5 })"#).unwrap_err();
        assert!(err.main.contains("open_on_output must be"), "{err}");
    }

    #[test]
    fn output_drm_lease() {
        let config = Config::parse_mem(
            r#"
            neori.output("HMD", { drm_lease = true })
            neori.output("DP-2", { ["drm-lease"] = true })
            neori.output("eDP-1", {})
            "#,
        )
        .unwrap();
        let outputs = &config.outputs.0;
        assert!(outputs[0].drm_lease);
        assert!(outputs[1].drm_lease);
        assert!(!outputs[2].drm_lease);
    }

    #[test]
    fn neori_api_named_output_and_workspace() {
        let config = Config::parse_mem(
            r#"
            neori.output("eDP-1", { scale = 2, focus_at_startup = true })
            neori.workspace("web", { open_on_output = "eDP-1" })
            "#,
        )
        .unwrap();

        assert_eq!(config.outputs.0.len(), 1);
        assert_eq!(config.outputs.0[0].name, "eDP-1");
        assert_eq!(config.outputs.0[0].scale, Some(FloatOrInt(2.0)));
        assert!(config.outputs.0[0].focus_at_startup);
        assert_eq!(config.workspaces.len(), 1);
        assert_eq!(config.workspaces[0].name.0, "web");
        assert_eq!(config.workspaces[0].open_on_output, ["eDP-1"]);

        let err = Config::parse_mem(r#"neori.window_rule("x", {})"#).unwrap_err();
        assert!(
            err.main.contains("expected a single table argument"),
            "{err}"
        );
        let err = Config::parse_mem(r#"neori.output(1, {})"#).unwrap_err();
        assert!(err.main.contains("name must be a string"), "{err}");
    }

    #[test]
    fn neori_api_output_fields() {
        let config = Config::parse_mem(
            r##"
            neori.output {
                name = "eDP-1",
                position = { x = 1280, y = 0 },
                max_bpc = 10,
                variable_refresh_rate = { on_demand = true },
                backdrop_color = "#001100",
                hot_corners = { top_left = true },
                mode = "1920x1080@120",
                modeline = {
                    clock = 173.0,
                    hdisplay = 1920, hsync_start = 2048, hsync_end = 2248, htotal = 2576,
                    vdisplay = 1080, vsync_start = 1083, vsync_end = 1088, vtotal = 1120,
                    hsync_polarity = "-hsync", vsync_polarity = "+vsync",
                },
            }
            "##,
        )
        .unwrap();

        let out = &config.outputs.0[0];
        assert_eq!(out.position, Some(Position { x: 1280, y: 0 }));
        assert_eq!(out.max_bpc, Some(MaxBpc(neori_ipc::MaxBpc::_10)));
        assert_eq!(out.variable_refresh_rate, Some(Vrr { on_demand: true }));
        assert!(out.backdrop_color.is_some());
        assert_eq!(out.hot_corners.unwrap().top_left, true);
        assert_eq!(
            out.mode,
            Some(Mode {
                custom: false,
                mode: neori_ipc::ConfiguredMode {
                    width: 1920,
                    height: 1080,
                    refresh: Some(120.0),
                },
            })
        );
        assert!(out.modeline.is_some());

        // Bare boolean VRR means always-on.
        let config =
            Config::parse_mem(r#"neori.output { name = "eDP-1", variable_refresh_rate = true }"#)
                .unwrap();
        assert_eq!(
            config.outputs.0[0].variable_refresh_rate,
            Some(Vrr { on_demand: false })
        );
    }

    #[test]
    fn output_create_virtual_flag_parses() {
        // snake_case
        let config = Config::parse_mem(
            r#"neori.output("sunshine", { create_virtual = true, mode = "1920x1080@144.000" })"#,
        )
        .unwrap();
        assert_eq!(config.outputs.0.len(), 1);
        assert_eq!(config.outputs.0[0].name, "sunshine");
        assert!(config.outputs.0[0].create_virtual);

        // kebab-case alias
        let config =
            Config::parse_mem(r#"neori.output { name = "HEADLESS-1", ["create-virtual"] = true }"#)
                .unwrap();
        assert!(config.outputs.0[0].create_virtual);

        // Default is false.
        let config = Config::parse_mem(r#"neori.output { name = "eDP-1" }"#).unwrap();
        assert!(!config.outputs.0[0].create_virtual);
    }

    #[test]
    fn max_bpc_rejects_out_of_range() {
        // Valid values parse.
        let config = Config::parse_mem(r#"neori.output { name = "eDP-1", max_bpc = 10 }"#).unwrap();
        assert_eq!(
            config.outputs.0[0].max_bpc,
            Some(MaxBpc(neori_ipc::MaxBpc::_10))
        );

        // Out-of-range integers must not wrap into a valid variant (262 as u8 == 6).
        let err =
            Config::parse_mem(r#"neori.output { name = "eDP-1", max_bpc = 262 }"#).unwrap_err();
        assert!(err.main.contains("max-bpc"), "{err}");

        let err = Config::parse_mem(r#"neori.output { name = "eDP-1", max_bpc = 7 }"#).unwrap_err();
        assert!(err.main.contains("max-bpc"), "{err}");
    }

    #[test]
    fn neori_api_animation_boolean_sugar() {
        let config = Config::parse_mem(
            r#"
            neori.animations {
                workspace_switch = false,
                window_open = { duration_ms = 150, curve = "ease-out-expo" },
            }
            "#,
        )
        .unwrap();

        assert!(config.animations.workspace_switch.0.off);
        assert!(!config.animations.window_open.anim.off);
    }

    #[test]
    fn neori_api_strict_mode() {
        // By default unknown keys are only warned about.
        let config = Config::parse_mem(r#"neori.layout { bogus_key = 1, gaps = 4 }"#).unwrap();
        assert_eq!(config.layout.gaps, 4.0);

        // The `neori.strict()` API turns unknown keys into errors.
        let err = Config::parse_mem(
            r#"
            neori.strict()
            neori.layout { bogus_key = 1 }
            "#,
        )
        .unwrap_err();
        assert!(
            err.main.contains("unknown key `bogus_key` in layout"),
            "{err}"
        );

        // `neori.strict(false)` disables it again.
        let config = Config::parse_mem(
            r#"
            neori.strict()
            neori.strict(false)
            neori.layout { bogus_key = 1, gaps = 4 }
            "#,
        )
        .unwrap();
        assert_eq!(config.layout.gaps, 4.0);
    }

    #[test]
    fn neori_api_include_glob_and_dir() {
        let dir = std::env::temp_dir().join(format!("neori-include-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();

        std::fs::write(
            dir.join("conf.d/10-binds.lua"),
            "neori.bind(\"Mod+Return\", \"spawn kitty\")",
        )
        .unwrap();
        std::fs::write(
            dir.join("conf.d/20-layout.lua"),
            "neori.layout { gaps = 12 }",
        )
        .unwrap();

        let config_path = dir.join("config.lua");
        std::fs::write(
            &config_path,
            "neori.include(\"conf.d/*.lua\")\nneori.include(\"conf.d\")\n",
        )
        .unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        let result = Config::parse(&config_path, &content);
        let config = result.config.expect("config should parse");

        assert_eq!(config.layout.gaps, 12.0);
        assert!(actions(&config).contains(&Action::Spawn {
            command: vec!["kitty".to_string()]
        }));
        // Included files are recorded once, for the config watcher.
        assert_eq!(result.includes.len(), 3);
        assert!(result.includes.contains(&dir.join("conf.d/10-binds.lua")));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fullscreen_backdrop_color() {
        let config = Config::parse_mem(
            r##"
            neori.layout { fullscreen_backdrop_color = "transparent" }
            neori.workspace("chat", { layout = { ["fullscreen-backdrop-color"] = "#ff000080" } })
            "##,
        )
        .unwrap();

        assert_eq!(
            config.layout.fullscreen_backdrop_color,
            Color::from_array_unpremul([0., 0., 0., 0.])
        );

        // Workspace layout overrides merge on top of the global layout.
        let part = &config.workspaces[0].layout.as_ref().unwrap().0;
        assert_eq!(
            part.fullscreen_backdrop_color,
            Some(Color::from_array_unpremul([1., 0., 0., 128. / 255.]))
        );
        let mut layout = config.layout.clone();
        crate::utils::MergeWith::merge_with(&mut layout, part);
        assert_eq!(
            layout.fullscreen_backdrop_color,
            Color::from_array_unpremul([1., 0., 0., 128. / 255.])
        );

        // Opaque black by default.
        let config = Config::parse_mem("").unwrap();
        assert_eq!(
            config.layout.fullscreen_backdrop_color,
            Color::from_array_unpremul([0., 0., 0., 1.])
        );
    }

    #[test]
    fn neori_api_merges_with_returned_table() {
        let config = Config::parse_mem(
            r#"
            neori.layout { gaps = 8 }
            neori.bind("Mod+B", "quit")
            return {
                layout = { gaps = 4 },
                binds = { ["Mod+A"] = "close_window" },
            }
            "#,
        )
        .unwrap();

        // The returned table wins, except that binds from both styles are kept.
        assert_eq!(config.layout.gaps, 4.0);
        assert_eq!(config.binds.0.len(), 2);
        let actions = actions(&config);
        assert!(actions.contains(&Action::Quit {
            skip_confirmation: false
        }));
        assert!(actions.contains(&Action::CloseWindow { id: None }));
    }

    #[test]
    fn neori_api_bind_errors() {
        // Lua function callbacks on binds are now supported, so this must parse.
        Config::parse_mem(r#"neori.bind("Mod+Q", function() end)"#).unwrap();

        let err = Config::parse_mem(
            r#"
            neori.bind("Mod+Q", "quit")
            neori.bind("Mod+Q", "close_window")
            "#,
        )
        .unwrap_err();
        assert!(err.main.contains("duplicate keybind"), "{err}");

        // Non-string/table/function action is still rejected.
        let err = Config::parse_mem(r#"neori.bind("Mod+Q", 42)"#).unwrap_err();
        assert!(
            err.main.contains("action must be") || err.main.contains("no action or Lua function"),
            "{err}"
        );
    }

    #[test]
    fn window_rule_empty_default_column_width() {
        let config = Config::parse_mem(
            r#"
            neori.window_rule {
                match = { app_id = "^org\\.wezfurlong\\.wezterm$" },
                default_column_width = {},
            }
            "#,
        )
        .unwrap();

        assert_eq!(
            config.window_rules[0].default_column_width,
            Some(DefaultPresetSize(None))
        );
    }

    #[test]
    fn custom_shader_inline() {
        let config = Config::parse_mem(
            r#"
            neori.animations {
                window_open = { duration_ms = 250, custom_shader = "open();" },
                window_close = { ["custom-shader"] = "close();" },
                window_resize = { custom_shader = [[resize();]] },
            }
            "#,
        )
        .unwrap();

        let anims = &config.animations;
        assert_eq!(anims.window_open.custom_shader.as_deref(), Some("open();"));
        assert_eq!(
            anims.window_close.custom_shader.as_deref(),
            Some("close();")
        );
        assert_eq!(
            anims.window_resize.custom_shader.as_deref(),
            Some("resize();")
        );
        // Other animation settings in the same table still apply.
        let crate::animations::Kind::Easing(params) = anims.window_open.anim.kind else {
            panic!("expected an easing animation");
        };
        assert_eq!(params.duration_ms, 250);

        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.animations.window_open.custom_shader, None);
    }

    #[test]
    fn custom_shader_from_path() {
        let dir = std::env::temp_dir().join(format!(
            "neori-config-shader-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(dir.join("shaders")).unwrap();
        let shader = dir.join("shaders/open.frag");
        std::fs::write(&shader, "from_file();").unwrap();
        let config_path = dir.join("config.lua");

        // Relative paths resolve against the config file's directory, and the shader file
        // gets watched for changes.
        let res = Config::parse(
            &config_path,
            r#"neori.animations { window_open = { custom_shader = { path = "shaders/open.frag" } } }"#,
        );
        let config = res.config.unwrap();
        assert_eq!(
            config.animations.window_open.custom_shader.as_deref(),
            Some("from_file();")
        );
        assert!(res.includes.contains(&shader));

        // Absolute paths work too.
        let config = Config::parse(
            &config_path,
            &format!(
                r#"neori.animations {{ window_close = {{ custom_shader = {{ path = "{}" }} }} }}"#,
                shader.display()
            ),
        )
        .config
        .unwrap();
        assert_eq!(
            config.animations.window_close.custom_shader.as_deref(),
            Some("from_file();")
        );

        // A missing file is an error, but stays watched so that creating it reloads.
        let missing = dir.join("missing.frag");
        let res = Config::parse(
            &config_path,
            r#"neori.animations { window_open = { custom_shader = { path = "missing.frag" } } }"#,
        );
        let err = res.config.unwrap_err();
        assert!(
            err.main.contains("failed to read custom shader"),
            "{}",
            err.main
        );
        assert!(res.includes.contains(&missing));

        // The table form requires a path.
        let res = Config::parse_mem(r#"neori.animations { window_open = { custom_shader = {} } }"#);
        assert!(res.unwrap_err().main.contains("custom shader path missing"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn xkb_file_is_read_and_watched() {
        let dir = std::env::temp_dir().join(format!(
            "neori-config-xkb-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let keymap = dir.join("keymap.xkb");
        std::fs::write(&keymap, "xkb_keymap {};").unwrap();
        let config_path = dir.join("config.lua");

        // Relative paths resolve against the config directory, and the file is watched.
        let res = Config::parse(
            &config_path,
            r#"neori.input { keyboard = { xkb = { file = "keymap.xkb" } } }"#,
        );
        let config = res.config.unwrap();
        assert_eq!(
            config.input.keyboard.xkb.file,
            Some(crate::input::XkbFile(Some("xkb_keymap {};".to_string())))
        );
        assert!(res.includes.contains(&keymap));

        // A missing file doesn't fail the config, but is still watched.
        let missing = dir.join("missing.xkb");
        let res = Config::parse(
            &config_path,
            r#"neori.input { keyboard = { xkb = { file = "missing.xkb" } } }"#,
        );
        assert_eq!(
            res.config.unwrap().input.keyboard.xkb.file,
            Some(crate::input::XkbFile(None))
        );
        assert!(res.includes.contains(&missing));

        // No file by default.
        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.input.keyboard.xkb.file, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn custom_shader_only_on_supported_animations() {
        // Lenient mode ignores the key on animations without shader support.
        let config = Config::parse_mem(
            r#"neori.animations { workspace_switch = { custom_shader = "x();" } }"#,
        )
        .unwrap();
        assert_eq!(config.animations.window_open.custom_shader, None);

        // Strict mode rejects it.
        assert!(Config::parse_mem(
            r#"
            neori.strict()
            neori.animations { workspace_switch = { custom_shader = "x();" } }
            "#,
        )
        .is_err());
    }

    #[test]
    fn focus_follows_mouse_outputs_only() {
        let parse = |text: &str| Config::parse_mem(text).unwrap().input.focus_follows_mouse;

        let ffm = parse("neori.input { focus_follows_mouse = { outputs_only = true } }").unwrap();
        assert_eq!(ffm.outputs_only, Some(true));

        let ffm =
            parse(r#"neori.input { focus_follows_mouse = { ["outputs-only"] = false } }"#).unwrap();
        assert_eq!(ffm.outputs_only, Some(false));

        let ffm = parse("neori.input { focus_follows_mouse = true }").unwrap();
        assert_eq!(ffm.outputs_only, None);
    }

    #[test]
    fn pinch_sensitivity() {
        let config = Config::parse_mem(
            r#"
            neori.input { touchpad = { pinch_sensitivity = 1.5 } }
            neori.window_rule { pinch_sensitivity = 1.25 }
            neori.window_rule { ["pinch-sensitivity"] = 2 }
            neori.window_rule { opacity = 0.5 }
            "#,
        )
        .unwrap();

        assert_eq!(
            config.input.touchpad.pinch_sensitivity,
            Some(FloatOrInt(1.5))
        );
        assert_eq!(
            config.window_rules[0].pinch_sensitivity,
            Some(FloatOrInt(1.25))
        );
        assert_eq!(
            config.window_rules[1].pinch_sensitivity,
            Some(FloatOrInt(2.))
        );
        assert_eq!(config.window_rules[2].pinch_sensitivity, None);

        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.input.touchpad.pinch_sensitivity, None);
    }

    #[test]
    fn pinch_sensitivity_out_of_range() {
        // Lenient mode warns and leaves the value unset.
        let config = Config::parse_mem(
            r#"
            neori.input { touchpad = { pinch_sensitivity = -1 } }
            neori.window_rule { pinch_sensitivity = 101 }
            "#,
        )
        .unwrap();
        assert_eq!(config.input.touchpad.pinch_sensitivity, None);
        assert_eq!(config.window_rules[0].pinch_sensitivity, None);

        // Strict mode rejects it.
        assert!(Config::parse_mem(
            r#"
            neori.strict()
            neori.window_rule { pinch_sensitivity = 101 }
            "#,
        )
        .is_err());
    }

    #[test]
    fn window_rule_variable_refresh_rate() {
        use crate::window_rule::VrrRule;

        let config = Config::parse_mem(
            r#"
            neori.window_rule { variable_refresh_rate = true }
            neori.window_rule { variable_refresh_rate = false }
            neori.window_rule { variable_refresh_rate = "fullscreen-only" }
            neori.window_rule { ["variable-refresh-rate"] = "true" }
            neori.window_rule { opacity = 0.5 }
            "#,
        )
        .unwrap();

        assert_eq!(
            config
                .window_rules
                .iter()
                .map(|r| r.variable_refresh_rate)
                .collect::<Vec<_>>(),
            [
                Some(VrrRule::On),
                Some(VrrRule::Off),
                Some(VrrRule::FullscreenOnly),
                Some(VrrRule::On),
                None,
            ]
        );

        assert!(Config::parse_mem(
            r#"
            neori.strict()
            neori.window_rule { variable_refresh_rate = "sometimes" }
            "#,
        )
        .is_err());
    }

    #[test]
    fn window_rule_open_on_output_fallbacks() {
        let config = Config::parse_mem(
            r#"
            neori.window_rule { open_on_output = { "DP-1", "DP-2" } }
            neori.window_rule { ["open-on-output"] = "eDP-1" }
            neori.window_rule { opacity = 0.5 }
            "#,
        )
        .unwrap();
        let rules = &config.window_rules;
        assert_eq!(rules[0].open_on_output, ["DP-1", "DP-2"]);
        assert_eq!(rules[1].open_on_output, ["eDP-1"]);
        assert!(rules[2].open_on_output.is_empty());
    }

    #[test]
    fn window_rule_on_xdg_activate() {
        let config = Config::parse_mem(
            r#"
            neori.window_rule { on_xdg_activate = "ignore" }
            neori.window_rule { ["on-xdg-activate"] = "set-urgent" }
            neori.window_rule { on_xdg_activate = "focus" }
            neori.window_rule { open_focused = true }
            "#,
        )
        .unwrap();

        assert_eq!(
            config
                .window_rules
                .iter()
                .map(|rule| rule.on_xdg_activate)
                .collect::<Vec<_>>(),
            vec![
                Some(OnXdgActivate::Ignore),
                Some(OnXdgActivate::SetUrgent),
                Some(OnXdgActivate::Focus),
                None,
            ]
        );
    }

    #[test]
    fn window_rule_on_xdg_activate_invalid_value() {
        // Lenient mode warns and leaves the rule unset.
        let config =
            Config::parse_mem(r#"neori.window_rule { on_xdg_activate = "steal" }"#).unwrap();
        assert_eq!(config.window_rules[0].on_xdg_activate, None);

        // Strict mode rejects the value.
        let err = Config::parse_mem(
            r#"
            neori.strict()
            neori.window_rule { on_xdg_activate = "steal" }
            "#,
        );
        assert!(err.is_err());
    }

    #[test]
    fn window_rule_geometry_corner_radius() {
        let config = Config::parse_mem(
            r#"
            neori.window_rule {
                draw_border_with_background = false,
                geometry_corner_radius = 10,
                clip_to_geometry = true,
            }
            "#,
        )
        .unwrap();

        let rule = &config.window_rules[0];
        assert_eq!(
            rule.geometry_corner_radius,
            Some(crate::appearance::CornerRadius::from(10.0))
        );
        assert_eq!(rule.clip_to_geometry, Some(true));
        assert_eq!(rule.draw_border_with_background, Some(false));

        let config =
            Config::parse_mem(r#"neori.window_rule { geometry_corner_radius = { 1, 2, 3, 4 } }"#)
                .unwrap();
        assert_eq!(
            config.window_rules[0].geometry_corner_radius,
            Some(crate::appearance::CornerRadius {
                top_left: 1.0,
                top_right: 2.0,
                bottom_right: 3.0,
                bottom_left: 4.0,
            })
        );
    }

    #[test]
    fn on_startup_function_callback() {
        let path = Path::new("test-config.lua");
        let (config, host) = Config::parse_with_host(
            path,
            r#"
            neori.on("startup", function()
                _G.started = true
            end)
            neori.bind("Mod+Return", "spawn kitty")
            "#,
        )
        .config
        .unwrap();

        assert!(config.startup.is_empty());
        assert!(host.read_global("started").unwrap().is_nil());
        host.call_event("startup").unwrap();
        assert!(host.read_global("started").unwrap().as_boolean().unwrap());
    }

    #[test]
    fn on_monitor_event_function_callback() {
        let path = Path::new("test-config.lua");
        let (config, host) = Config::parse_with_host(
            path,
            r#"
            neori.on("monitor-added", function(name)
                _G.added = name == "HDMI-A-1"
            end)
            neori.on("monitor-removed", function(name)
                _G.removed = name == "eDP-1"
            end)
            "#,
        )
        .config
        .unwrap();

        assert!(config.monitor_added.is_empty());
        assert!(host.read_global("added").unwrap().is_nil());

        host.call_event_with("monitor-added", "HDMI-A-1".to_string())
            .unwrap();
        host.call_event_with("monitor-removed", "eDP-1".to_string())
            .unwrap();

        assert!(host.read_global("added").unwrap().as_boolean().unwrap());
        assert!(host.read_global("removed").unwrap().as_boolean().unwrap());
    }

    #[test]
    fn on_switch_event_function_callback() {
        let path = Path::new("test-config.lua");
        let (config, host) = Config::parse_with_host(
            path,
            r#"
            neori.on("lid-open", function()
                _G.lid_opened = true
            end)
            neori.on("lid-closed", function()
                _G.lid_closed = true
            end)
            neori.on("tablet-mode-on", function()
                _G.tablet_on = true
            end)
            neori.on("tablet-mode-off", function()
                _G.tablet_off = true
            end)
            "#,
        )
        .config
        .unwrap();

        assert!(config.lid_open.is_empty());
        assert!(config.lid_closed.is_empty());
        assert!(config.tablet_mode_on.is_empty());
        assert!(config.tablet_mode_off.is_empty());

        host.call_event("lid-open").unwrap();
        host.call_event("lid-closed").unwrap();
        host.call_event("tablet-mode-on").unwrap();
        host.call_event("tablet-mode-off").unwrap();

        assert!(host
            .read_global("lid_opened")
            .unwrap()
            .as_boolean()
            .unwrap());
        assert!(host
            .read_global("lid_closed")
            .unwrap()
            .as_boolean()
            .unwrap());
        assert!(host.read_global("tablet_on").unwrap().as_boolean().unwrap());
        assert!(host
            .read_global("tablet_off")
            .unwrap()
            .as_boolean()
            .unwrap());
    }

    #[test]
    fn switch_event_actions_parse() {
        let config = Config::parse_mem(
            r#"
            neori.on("lid-open", "spawn waymon -a")
            neori.on("lid-closed", "spawn waymon -a")
            neori.on("tablet-mode-on", "spawn tablet-on")
            neori.on("tablet-mode-off", "spawn tablet-off")
            "#,
        )
        .unwrap();

        assert_eq!(config.lid_open.len(), 1);
        assert_eq!(config.lid_closed.len(), 1);
        assert_eq!(config.tablet_mode_on.len(), 1);
        assert_eq!(config.tablet_mode_off.len(), 1);

        assert_eq!(
            config.lid_open[0],
            Action::Spawn {
                command: vec!["waymon -a".to_string()]
            }
        );
    }

    #[test]
    fn on_startup_function_callback_in_include() {
        let dir = std::env::temp_dir().join(format!("neori-on-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let helper_path = dir.join("helper.lua");
        let config_path = dir.join("test-config.lua");

        std::fs::write(
            &helper_path,
            "neori.on(\"startup\", function() _G.helper_ran = true end)",
        )
        .unwrap();
        std::fs::write(
            &config_path,
            "require \"helper\"\nreturn { prefer_no_csd = true }\n",
        )
        .unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        let (config, host) = Config::parse_with_host(&config_path, &content)
            .config
            .expect("config should parse");

        assert!(config.prefer_no_csd);
        assert!(host.read_global("helper_ran").unwrap().is_nil());
        host.call_event("startup").unwrap();
        assert!(host
            .read_global("helper_ran")
            .unwrap()
            .as_boolean()
            .unwrap());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn on_startup_unknown_event_is_an_error() {
        let err = Config::parse_mem(r#"neori.on("nope", "spawn waybar")"#).unwrap_err();
        assert!(
            err.main.contains("unknown event 'nope'"),
            "error should name the unknown event: {err}"
        );
        // The error message enumerates every supported event; spot-check that the
        // window-* events are listed so stale configs catch typos against the current set.
        for name in [
            "startup",
            "monitor-added",
            "monitor-removed",
            "lid-open",
            "lid-closed",
            "tablet-mode-on",
            "tablet-mode-off",
            "window-opened",
            "window-closed",
            "window-focused",
        ] {
            assert!(
                err.main.contains(name),
                "error should mention supported event {name:?}: {err}"
            );
        }
    }

    #[test]
    fn on_startup_bad_handler_is_an_error() {
        let err = Config::parse_mem(r#"neori.on("startup", 42)"#).unwrap_err();
        assert!(
            err.main
                .contains("event handler must be a string, a table, or a function"),
            "{err}"
        );
    }

    #[test]
    fn on_startup_table_handler_without_action_is_an_error() {
        let err = Config::parse_mem(r#"neori.on("startup", { cooldown_ms = 100 })"#).unwrap_err();
        assert!(
            err.main
                .contains("startup handler table must contain an action"),
            "{err}"
        );
    }
}
