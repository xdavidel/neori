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
pub use crate::layer_rule::LayerRule;
pub use crate::layout::*;
pub use crate::misc::*;
pub use crate::output::{Output, OutputName, Outputs, Position, Vrr};
pub use crate::parse_lua::ScriptHost;
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
        assert!(actions.contains(&Action::Spawn(vec!["kitty".to_string()])));

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
        +        Spawn(
        +            [
        +                "waybar",
        +            ],
        +        ),
        +    ],
        "#,
        );
    }

    fn actions(config: &Config) -> Vec<Action> {
        config.binds.0.iter().map(|b| b.action.clone()).collect()
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
        assert_eq!(config.startup, [Action::Spawn(vec!["waybar".to_string()])]);
        assert_eq!(config.binds.0.len(), 2);
        let actions = actions(&config);
        assert!(actions.contains(&Action::Spawn(vec!["kitty".to_string()])));
        assert!(actions.contains(&Action::FocusWorkspace(WorkspaceReference::Index(1))));
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
                Action::Spawn(vec!["waybar".to_string()]),
                Action::Spawn(vec!["waybar2".to_string()]),
            ]
        );
        assert_eq!(
            config.monitor_removed,
            [
                Action::Spawn(vec!["cleanup".to_string()]),
                Action::SpawnSh("echo bye".to_string()),
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

        assert_eq!(config.startup, [Action::Spawn(vec!["waybar".to_string()])]);
        let actions = actions(&config);
        assert!(actions.contains(&Action::Spawn(vec!["kitty".to_string()])));
        assert!(actions.contains(&Action::Spawn(vec![
            "kitty".to_string(),
            "-e".to_string(),
            "sh".to_string()
        ])));
        assert!(actions.contains(&Action::Spawn(vec![
            "brightnessctl".to_string(),
            "set".to_string(),
            "+10%".to_string()
        ])));
        assert!(actions.contains(&Action::SpawnSh("wl-mirror $(echo hi)".to_string())));

        let brightness = config
            .binds
            .0
            .iter()
            .find(|b| {
                b.action
                    == Action::Spawn(vec![
                        "brightnessctl".to_string(),
                        "set".to_string(),
                        "+10%".to_string(),
                    ])
            })
            .unwrap();
        assert!(brightness.allow_when_locked);

        let meta = config
            .binds
            .0
            .iter()
            .find(|b| b.action == Action::Spawn(vec!["terminal-meta".to_string()]))
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
        assert!(actions.contains(&Action::FocusColumnFirst));
        assert!(actions.contains(&Action::MoveColumnToMonitorLeft));
        assert!(actions.contains(&Action::MoveWorkspaceDown));
        assert!(actions.contains(&Action::ConsumeOrExpelWindowRight));
        assert!(actions.contains(&Action::SwitchPresetWindowHeight));
        assert!(actions.contains(&Action::ResetWindowHeight));
        assert!(actions.contains(&Action::ExpandColumnToAvailableWidth));
        assert!(actions.contains(&Action::CenterVisibleColumns));
        assert!(actions.contains(&Action::SwitchFocusBetweenFloatingAndTiling));
        assert!(actions.contains(&Action::MoveColumnToWorkspaceDown(true)));
        assert!(actions.contains(&Action::SetColumnWidth(
            neori_ipc::SizeChange::AdjustProportion(-10.0)
        )));
        assert!(actions.contains(&Action::SetWindowHeight(
            neori_ipc::SizeChange::AdjustProportion(10.0)
        )));
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
        assert!(actions.contains(&Action::DoScreenTransition(Some(100))));
        assert!(actions.contains(&Action::ToggleWindowRuleOpacity));
        assert!(actions.contains(&Action::ToggleDebugTint));
        assert!(actions.contains(&Action::DebugToggleDamage));
        assert!(actions.contains(&Action::DebugToggleOpaqueRegions));
        assert!(actions.contains(&Action::Quit(true)));
        assert!(actions.contains(&Action::Screenshot(false, None)));
        assert!(actions.contains(&Action::ScreenshotScreen(false, false, None)));
        assert!(actions.contains(&Action::ScreenshotWindow(false, true, None)));
        assert!(actions.contains(&Action::DoScreenTransition(None)));
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
        assert_eq!(config.startup, [Action::SpawnSh("qs -c shell".to_string())]);
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
        assert_eq!(config.workspaces[0].open_on_output.as_deref(), Some("eDP-1"));

        let err = Config::parse_mem(r#"neori.window_rule("x", {})"#).unwrap_err();
        assert!(err.main.contains("expected a single table argument"), "{err}");
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
        let config = Config::parse_mem(
            r#"neori.output { name = "eDP-1", variable_refresh_rate = true }"#,
        )
        .unwrap();
        assert_eq!(
            config.outputs.0[0].variable_refresh_rate,
            Some(Vrr { on_demand: false })
        );
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
        assert!(err.main.contains("unknown key `bogus_key` in layout"), "{err}");

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
        assert!(actions(&config).contains(&Action::Spawn(vec!["kitty".to_string()])));
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
        assert!(actions.contains(&Action::Quit(false)));
        assert!(actions.contains(&Action::CloseWindow));
    }

    #[test]
    fn neori_api_bind_errors() {
        let err = Config::parse_mem(r#"neori.bind("Mod+Q", function() end)"#).unwrap_err();
        assert!(
            err.main.contains("function actions are not supported yet"),
            "{err}"
        );

        let err = Config::parse_mem(
            r#"
            neori.bind("Mod+Q", "quit")
            neori.bind("Mod+Q", "close_window")
            "#,
        )
        .unwrap_err();
        assert!(err.main.contains("duplicate keybind"), "{err}");
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
    fn on_startup_function_callback_in_include() {
        use std::io::Write as _;

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
            err.main.contains(
                "unknown event 'nope' (supported events: startup, monitor-added, monitor-removed)"
            ),
            "{err}"
        );
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
