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
pub mod utils;
pub mod window_rule;
pub mod workspace;

pub use crate::animations::{Animation, Animations};
pub use crate::appearance::*;
pub use crate::binds::*;
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
    LuaOutput, LuaWorkspace, OutputCache, OutputSnapshot, ScriptHost, Window, WindowCache,
    WindowSnapshot, WorkspaceCache, WorkspaceSnapshot,
};
/// Re-exported so downstream crates (compositor, tests) can read Lua values via
/// [`ScriptHost::read_global`] without taking a direct dependency on `mlua`.
pub use mlua;
pub use crate::recent_windows::{MruDirection, MruFilter, MruPreviews, MruScope, RecentWindows};
pub use crate::utils::FloatOrInt;
pub use crate::window_rule::{
    FloatingPosition, PopupsRule, RelativeTo, ResolvedPopupsRules, WindowRule,
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
            modes: vec!["1920x1080@60.000".to_string(), "1280x720@60.000".to_string()],
            scale: Some(1.25),
            transform: Some("90".to_string()),
            position: Some((100, 200)),
            is_focused: true,
            is_virtual: true,
            off: false,
            vrr_supported: true,
            vrr_enabled: false,
        };
        host.set_output_cache(OutputCache {
            outputs: vec![snap.clone()],
            focused: Some(snap),
        });
        host.call_event("startup").unwrap();

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
    fn output_mutation_methods_enqueue_ops() {
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
        host.set_output_cache(OutputCache {
            outputs: vec![snap.clone()],
            focused: Some(snap),
        });
        host.call_event("startup").unwrap();

        let ops = host.drain_pending_output_ops();
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
                "off", "on", "off", "mode", "mode", "scale", "scale", "transform", "position",
                "position", "vrr",
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
        host.set_workspace_cache(WorkspaceCache {
            workspaces: vec![ws_snap.clone()],
            focused: Some(ws_snap),
        });
        // ws:windows() filters this cache by window_ids.
        host.set_window_cache(WindowCache {
            windows: vec![
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
            focused: None,
        });
        host.call_event("startup").unwrap();

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
        // First match must be 42 (window_ids order: [42, 7]; filter iterates window cache
        // but either order is acceptable — just assert we see one of them).
        let fw: u64 = t.get("first_window_id").unwrap();
        assert!(fw == 42 || fw == 7, "unexpected first window id {fw}");
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
    fn workspace_mutation_methods_enqueue_actions() {
        // Each mutation method should push its matching Action onto the pending-actions
        // queue. Drain via call_bind_function.
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
        host.set_workspace_cache(WorkspaceCache {
            workspaces: vec![ws_snap.clone()],
            focused: Some(ws_snap),
        });

        let actions = host.call_bind_function(1).unwrap();
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
            "unexpected enqueued actions: {kinds:?}"
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
    fn window_mutation_methods_enqueue_actions() {
        // Every mutation method should push its matching Action onto the pending-actions
        // queue. call_bind_function drains that queue and returns the collected actions
        // so we can assert on them here.
        let (_config, host) = Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            neori.bind("Mod+Alt+T", function()
                local w = neori.focused_window()
                w:close()
                w:focus()
                w:fullscreen(true)   -- snapshot has is_fullscreen=false, should enqueue
                w:fullscreen(false)  -- already false in snapshot, no-op
                w:fullscreen()       -- always enqueue
                w:float(true)        -- snapshot is_floating=false, enqueues MoveToFloating
                w:float(false)       -- snapshot is_floating=false, no-op
                w:send_to_workspace(2)
                w:send_to_workspace("browser")
                w:send_to_output("eDP-1")
            end)
            "#,
        )
        .config
        .unwrap();

        // Seed a snapshot: is_fullscreen=false, is_floating=false.
        let snap = WindowSnapshot {
            id: 42,
            is_fullscreen: false,
            is_floating: false,
            ..Default::default()
        };
        host.set_window_cache(WindowCache {
            windows: vec![snap.clone()],
            focused: Some(snap),
        });

        let actions = host.call_bind_function(1).unwrap();
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
        // The two no-op calls (w:fullscreen(false), w:float(false)) must NOT enqueue.
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
            "unexpected enqueued actions: {kinds:?}"
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
    fn window_mutation_on_stale_id_still_enqueues() {
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
        host.set_window_cache(WindowCache {
            windows: vec![snap.clone()],
            focused: Some(snap),
        });

        let actions = host.call_bind_function(1).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::CloseWindow { id: Some(99) }));

        // Second call: snapshot isn't refreshed, but Lua `stashed` still holds the
        // original Window userdata. Call it again — must still enqueue the same action
        // without erroring.
        host.set_window_cache(WindowCache::default());
        let actions = host.call_bind_function(1).unwrap();
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
            // call_event_with returns Ok(empty Vec) for events with no state. The
            // registration was successful if the call doesn't error.
            let actions = host
                .call_event_with::<Window>(ev, Window(WindowSnapshot::default()))
                .unwrap();
            // The default snapshot runs through the handler without side effects.
            assert!(actions.is_empty(), "{ev}: unexpected actions {actions:?}");
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
            workspace_idx: Some(3),
            workspace_name: Some("browser".to_string()),
            output_name: Some("eDP-1".to_string()),
        };
        host.call_event_with("window-focused", Window(snap)).unwrap();

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
    fn neori_windows_reads_cached_snapshots() {
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
        host.set_window_cache(WindowCache {
            windows: vec![w1, w2.clone()],
            focused: Some(w2),
        });
        host.call_event("startup").unwrap();

        let seen = host.read_global("seen").unwrap();
        assert!(matches!(seen, mlua::Value::Integer(2)));
        let app = host.read_global("focused_app_id").unwrap();
        let mlua::Value::String(s) = app else {
            panic!("focused_app_id is not a string");
        };
        assert_eq!(s.to_str().unwrap().to_string(), "b");
    }

    #[test]
    fn lua_function_bind_parses_as_script_fn() {
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

        // Exactly one bind should be a Lua-function bind (no static action, has script_fn)
        // and one should be a string-action bind (has action, no script_fn).
        let fn_binds: Vec<_> = config
            .binds
            .0
            .iter()
            .filter(|b| b.script_fn.is_some())
            .collect();
        assert_eq!(fn_binds.len(), 1, "expected one function bind");
        let fn_bind = fn_binds[0];
        assert!(
            fn_bind.action.is_none(),
            "function bind should have no static Action, got {:?}",
            fn_bind.action
        );
        let script_fn = fn_bind.script_fn.as_ref().unwrap();
        assert!(
            host.has_bind_function(script_fn.id),
            "ScriptHost should have the function registered for id {}",
            script_fn.id
        );

        // The other bind carries the static Action.
        let str_bind = config
            .binds
            .0
            .iter()
            .find(|b| b.script_fn.is_none())
            .expect("string-action bind not found");
        assert_eq!(str_bind.action, Some(Action::CloseWindow { id: None }));
    }

    #[test]
    fn neori_action_enqueues_pending_action() {
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

        // Trigger the Lua function; neori.action queued three actions.
        // The first bind function registered in this host gets id 1.
        let actions = host.call_bind_function(1).unwrap();
        assert_eq!(actions.len(), 3, "got {actions:?}");
        assert!(matches!(actions[0], Action::Spawn { .. }));
        assert!(matches!(actions[1], Action::CloseWindow { id: None }));
        assert!(matches!(actions[2], Action::FocusColumnLeft {}));

        // Draining again yields nothing (consumed).
        assert!(host.drain_pending_actions().is_empty());
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
        assert!(actions.contains(&Action::SendToScratchpad {}));
        assert!(actions.contains(&Action::ToggleScratchpad {}));
        assert!(actions.contains(&Action::RemoveFromScratchpad {}));
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
        config.binds.0.iter().filter_map(|b| b.action.clone()).collect()
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
                b.action
                    == Some(Action::Spawn {
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
                b.action
                    == Some(Action::Spawn {
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
            Some(BlockOutFrom::ScreenCapture)
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
        assert_eq!(
            config.workspaces[0].open_on_output.as_deref(),
            Some("eDP-1")
        );

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
        let config = Config::parse_mem(
            r#"neori.output { name = "HEADLESS-1", ["create-virtual"] = true }"#,
        )
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
            err.main.contains("action must be")
                || err.main.contains("no action or Lua function"),
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
