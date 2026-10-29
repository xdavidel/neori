use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mlua::prelude::*;
use mlua::Value;
use neori_ipc::{ColumnDisplay, ConfiguredMode, Layer, SizeChange, Transform};

use crate::animations::*;
use crate::appearance::*;
use crate::binds::*;
use crate::gestures::*;
use crate::input::*;
use crate::layout::*;
use crate::misc::*;
use crate::output::*;
use crate::utils::{Flag, Percent, RegexEq};
use crate::window_rule::*;
use crate::workspace::*;
use crate::*;

pub struct ParsedConfig {
    pub config: Config,
    pub includes: Vec<PathBuf>,
    /// The Lua host that parsed the config, kept alive so event callbacks can run later.
    ///
    /// Only filled by [`parse_lua_with_host`]; [`parse_lua`] leaves the default host in
    /// place (it is not used by the caller).
    pub host: ScriptHost,
}

pub struct ParseLuaError {
    pub message: String,
    pub includes: Vec<PathBuf>,
}

/// The registry of Lua function callbacks registered via `neori.on(event, function)`.
type EventRegistry = Arc<Mutex<HashMap<String, Vec<mlua::Function>>>>;

/// A sink that captures Lua `print(...)` output. The compositor installs one
/// at startup pointing at a log file; tests can install an in-memory buffer.
pub type PrintSink = Arc<Mutex<Box<dyn Write + Send>>>;

/// Shared slot holding the currently installed [`PrintSink`]. The `print`
/// override reads this on each call so a sink can be installed after the Lua
/// state has already been built.
type PrintSinkSlot = Arc<Mutex<Option<PrintSink>>>;

/// The registry of Lua function callbacks registered via `neori.bind(key, function)`.
///
/// Keyed by the [`Action::ScriptFunction`] id assigned at parse time.
type BindFnRegistry = Arc<Mutex<HashMap<u64, mlua::Function>>>;

/// Queue of [`Action`]s that Lua code enqueued via `neori.action(name, args)`.
///
/// The compositor drains it after every Lua callback call and dispatches each action.
type PendingActions = Arc<Mutex<Vec<Action>>>;

/// Snapshot of a window's state, exposed to Lua as the `Window` userdata.
///
/// All fields are captured at the moment the snapshot is built; subsequent compositor
/// changes do not update a snapshot that has already been handed to Lua. This keeps the
/// Lua/Rust boundary simple (no lifetime games, no "window gone" errors) at the cost of
/// requiring users to re-query via `neori.windows()` or wait for the next event.
#[derive(Debug, Clone, Default)]
pub struct WindowSnapshot {
    /// Stable per-window id (`MappedId::get()`). Carries forward to the mutation API
    /// planned for the next scripting round.
    pub id: u64,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_fullscreen: bool,
    /// 1-based global index of the workspace the window lives on.
    pub workspace_idx: Option<u8>,
    pub workspace_name: Option<String>,
    /// Connector-style output name the window currently lives on.
    pub output_name: Option<String>,
}

/// Cached snapshot of compositor state used by `neori.windows()` and
/// `neori.focused_window()`. The compositor refreshes it before every Lua callback.
#[derive(Debug, Default, Clone)]
pub struct WindowCache {
    pub windows: Vec<WindowSnapshot>,
    pub focused: Option<WindowSnapshot>,
}

type SharedWindowCache = Arc<Mutex<WindowCache>>;

/// Snapshot of an output's state, exposed to Lua as the `Output` userdata.
///
/// Same semantics as [`WindowSnapshot`]: values are captured at the moment the
/// snapshot is built; future compositor changes do not mutate a snapshot already in
/// Lua hands. Re-query via [`neori.outputs()`](Self) or `neori.focused_output()` to
/// see fresh state.
#[derive(Debug, Clone, Default)]
pub struct OutputSnapshot {
    /// Connector-style name (e.g. "eDP-1", "HEADLESS-1"). Stable across the output's
    /// lifetime.
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// Current mode as a string "WxH@REFRESH" (e.g. "1920x1080@60.000"), or `None` if
    /// the output has no current mode.
    pub mode: Option<String>,
    /// Available modes as strings in the same "WxH@REFRESH" format.
    pub modes: Vec<String>,
    pub scale: Option<f64>,
    /// Transform as a Lua-side string: "normal" | "90" | "180" | "270" | "flipped" |
    /// "flipped-90" | "flipped-180" | "flipped-270".
    pub transform: Option<String>,
    pub position: Option<(i32, i32)>,
    pub is_focused: bool,
    /// Whether the output is virtual (HEADLESS-*). Mirrors `VirtualOutputMarker`.
    pub is_virtual: bool,
    /// Whether the output is currently off in the compositor's view.
    pub off: bool,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
}

#[derive(Debug, Default, Clone)]
pub struct OutputCache {
    pub outputs: Vec<OutputSnapshot>,
    pub focused: Option<OutputSnapshot>,
}

type SharedOutputCache = Arc<Mutex<OutputCache>>;

/// Snapshot of a workspace's state, exposed to Lua as the `Workspace` userdata.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSnapshot {
    /// Global 1-based index (same shape as `Window:workspace().idx`).
    pub idx: u8,
    pub name: Option<String>,
    pub output_name: Option<String>,
    pub is_focused: bool,
    /// Stable per-session workspace id, mirrored from the compositor's
    /// `WorkspaceId`. Used by the Window userdata's `w:workspace()` lookup when a
    /// Lua caller wants to correlate.
    pub id: u64,
    /// Ids of windows on this workspace, matching [`WindowSnapshot::id`]. Used by
    /// `ws:windows()` to filter the shared window cache.
    pub window_ids: Vec<u64>,
}

#[derive(Debug, Default, Clone)]
pub struct WorkspaceCache {
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub focused: Option<WorkspaceSnapshot>,
}

type SharedWorkspaceCache = Arc<Mutex<WorkspaceCache>>;

/// Pointer-position snapshot exposed to Lua as `neori.pointer()`. The compositor
/// refreshes it in `refresh_script_caches` from the seat's current pointer
/// location, so Lua sees fresh values at every bind / event boundary.
#[derive(Debug, Default, Clone)]
pub struct PointerSnapshot {
    pub x: f64,
    pub y: f64,
    /// Connector name of the output the pointer currently sits on, or `None` if
    /// it's in a gap between outputs (rare — usually it's on exactly one).
    pub output_name: Option<String>,
}

/// Snapshot of one libinput device, exposed to Lua as the `Device` userdata.
/// The compositor refreshes a cache in `refresh_script_caches` so
/// `neori.devices()` sees fresh metadata at every callback boundary.
#[derive(Debug, Default, Clone)]
pub struct DeviceSnapshot {
    /// libinput device name (e.g. `"AT Translated Set 2 keyboard"`).
    pub name: String,
    /// Stable per-boot udev sysname (e.g. `"event5"`). Used as the mutation
    /// target key: mutation methods enqueue `(sysname, DeviceOp)` tuples which
    /// the compositor dispatches by looking the device up in `neori.devices`.
    pub sysname: String,
    pub product_id: u32,
    pub vendor_id: u32,
    pub is_pointer: bool,
    pub is_keyboard: bool,
    pub is_touch: bool,
    /// True for touchpads specifically (any pointer device with a non-zero
    /// `config_tap_finger_count`), as opposed to mice / trackballs.
    pub is_touchpad: bool,
}

#[derive(Debug, Default, Clone)]
pub struct DeviceCache {
    pub devices: Vec<DeviceSnapshot>,
}

type SharedDeviceCache = Arc<Mutex<DeviceCache>>;

/// Runtime mutations enqueued against a specific device by its sysname.
/// Drained by the compositor after the callback returns; each op maps onto
/// a `device.config_*_set_*` libinput call.
///
/// Changes persist on the device until the next `apply_libinput_settings`
/// (i.e. config reload), at which point the config value takes over again.
#[derive(Debug, Clone)]
pub enum DeviceOp {
    SetTap(bool),
    SetNaturalScroll(bool),
    SetAccel(f64),
    SetLeftHanded(bool),
    SetDisabled(bool),
}

type PendingDeviceOps = Arc<Mutex<Vec<(String, DeviceOp)>>>;

type SharedPointerCache = Arc<Mutex<Option<PointerSnapshot>>>;

/// Queue of pending cursor warps enqueued by Lua via `neori.warp(x, y)`. The
/// compositor drains and dispatches each via `State::move_cursor` after the
/// callback returns.
type PendingWarps = Arc<Mutex<Vec<(f64, f64)>>>;

/// Current keyboard layout name exposed to Lua as `neori.keyboard_layout()`.
/// The compositor keeps this fresh from the seat's xkb state.
type SharedKeyboardLayout = Arc<Mutex<Option<String>>>;

/// Queue of output configuration operations enqueued by Lua via the `Output`
/// userdata mutation methods. The compositor drains and dispatches each via
/// `State::apply_transient_output_config` after the callback returns.
type PendingOutputOps = Arc<Mutex<Vec<(String, neori_ipc::OutputAction)>>>;

/// Pre-placement overrides a Lua `window-opening` handler can set on the window
/// via the `w:open_*` methods. The compositor reads these just before calling
/// `Layout::add_window` and merges them into the per-window
/// `ResolvedWindowRules` (overrides win). Fields that stay `None` leave the
/// compositor-resolved rule untouched.
///
/// Access is guarded by `Arc<Mutex<Option<OpenOverrides>>>`: the compositor
/// sets the inner to `Some(default)` just before firing `window-opening` and
/// takes it back to `None` right after draining. A Lua `w:open_*` call outside
/// that window is a silent no-op — the guard is `None`, so writes are dropped.
/// This keeps a stashed `Window` from, say, a startup bind unable to leak an
/// override onto some unrelated future window that opens.
#[derive(Debug, Default, Clone)]
pub struct OpenOverrides {
    pub open_on_output: Option<String>,
    pub open_on_workspace: Option<String>,
    pub open_fullscreen: Option<bool>,
    pub open_maximized: Option<bool>,
    pub open_floating: Option<bool>,
}

type SharedOpenOverrides = Arc<Mutex<Option<OpenOverrides>>>;

/// Lua `Output` userdata: a snapshot wrapper with read-only accessors and
/// mutation methods that enqueue `OutputAction`s against this output by name.
///
/// Named `LuaOutput` on the Rust side to disambiguate from
/// [`crate::output::Output`] (the config-time struct). Lua code just sees an
/// Output userdata.
#[derive(Debug, Clone)]
pub struct LuaOutput(pub OutputSnapshot);

impl mlua::UserData for LuaOutput {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // Read-only accessors.
        methods.add_method("name", |_, this, ()| Ok(this.0.name.clone()));
        methods.add_method("make", |_, this, ()| Ok(this.0.make.clone()));
        methods.add_method("model", |_, this, ()| Ok(this.0.model.clone()));
        methods.add_method("serial", |_, this, ()| Ok(this.0.serial.clone()));
        methods.add_method("mode", |_, this, ()| Ok(this.0.mode.clone()));
        methods.add_method("modes", |lua, this, ()| {
            let t = lua.create_table()?;
            for (i, m) in this.0.modes.iter().enumerate() {
                t.set(i + 1, m.clone())?;
            }
            Ok(t)
        });
        methods.add_method("scale", |_, this, ()| Ok(this.0.scale));
        methods.add_method("transform", |_, this, ()| Ok(this.0.transform.clone()));
        methods.add_method("position", |lua, this, ()| match this.0.position {
            Some((x, y)) => {
                let t = lua.create_table()?;
                t.set("x", x)?;
                t.set("y", y)?;
                Ok(mlua::Value::Table(t))
            }
            None => Ok(mlua::Value::Nil),
        });
        methods.add_method("is_focused", |_, this, ()| Ok(this.0.is_focused));
        methods.add_method("is_virtual", |_, this, ()| Ok(this.0.is_virtual));
        methods.add_method("is_off", |_, this, ()| Ok(this.0.off));
        methods.add_method("vrr_supported", |_, this, ()| Ok(this.0.vrr_supported));
        methods.add_method("vrr_enabled", |_, this, ()| Ok(this.0.vrr_enabled));

        // Mutation methods. Each enqueues an (output name, OutputAction) pair which
        // the compositor drains and dispatches via apply_transient_output_config.

        // out:on() / out:off() / out:enable(bool)
        methods.add_method("on", |lua, this, ()| {
            enqueue_output_op(lua, &this.0.name, neori_ipc::OutputAction::On);
            Ok(())
        });
        methods.add_method("off", |lua, this, ()| {
            enqueue_output_op(lua, &this.0.name, neori_ipc::OutputAction::Off);
            Ok(())
        });
        methods.add_method("enable", |lua, this, enable: Option<bool>| {
            // Default: flip to the opposite of the current state (snapshot-based).
            let desired = enable.unwrap_or(this.0.off);
            // Skip if the snapshot already matches — same idempotence rules as
            // Window:fullscreen(bool). `off` means "not enabled", so the match is
            // `desired != off`.
            if desired != this.0.off {
                return Ok(());
            }
            let op = if desired {
                neori_ipc::OutputAction::On
            } else {
                neori_ipc::OutputAction::Off
            };
            enqueue_output_op(lua, &this.0.name, op);
            Ok(())
        });

        // out:set_mode("1920x1080@144") or out:set_mode("auto")
        methods.add_method("set_mode", |lua, this, mode: String| {
            let op = if mode == "auto" || mode.is_empty() {
                neori_ipc::OutputAction::Mode {
                    mode: neori_ipc::ModeToSet::Automatic,
                }
            } else {
                let configured = parse_mode_string_strict(&mode).map_err(LuaError::runtime)?;
                neori_ipc::OutputAction::Mode {
                    mode: neori_ipc::ModeToSet::Specific(configured),
                }
            };
            enqueue_output_op(lua, &this.0.name, op);
            Ok(())
        });

        // out:set_scale(1.5) / out:set_scale("auto")
        methods.add_method("set_scale", |lua, this, scale: mlua::Value| {
            let op = match &scale {
                mlua::Value::String(s) => {
                    let s = s.to_str().map_err(|e| LuaError::runtime(e.to_string()))?;
                    if s == "auto" {
                        neori_ipc::OutputAction::Scale {
                            scale: neori_ipc::ScaleToSet::Automatic,
                        }
                    } else {
                        return Err(LuaError::runtime(format!(
                            "set_scale: expected a number or \"auto\", got \"{s}\""
                        )));
                    }
                }
                mlua::Value::Number(n) => neori_ipc::OutputAction::Scale {
                    scale: neori_ipc::ScaleToSet::Specific(*n),
                },
                mlua::Value::Integer(i) => neori_ipc::OutputAction::Scale {
                    scale: neori_ipc::ScaleToSet::Specific(*i as f64),
                },
                other => {
                    return Err(LuaError::runtime(format!(
                        "set_scale: expected a number or \"auto\", got {}",
                        other.type_name()
                    )))
                }
            };
            enqueue_output_op(lua, &this.0.name, op);
            Ok(())
        });

        // out:set_transform("90") etc.
        methods.add_method("set_transform", |lua, this, s: String| {
            let transform = parse_transform_string(&s).map_err(LuaError::runtime)?;
            enqueue_output_op(
                lua,
                &this.0.name,
                neori_ipc::OutputAction::Transform { transform },
            );
            Ok(())
        });

        // out:set_position({x=…, y=…}) / out:set_position("auto")
        methods.add_method("set_position", |lua, this, value: mlua::Value| {
            let op = match &value {
                mlua::Value::Nil => neori_ipc::OutputAction::Position {
                    position: neori_ipc::PositionToSet::Automatic,
                },
                mlua::Value::String(s) => {
                    let s = s.to_str().map_err(|e| LuaError::runtime(e.to_string()))?;
                    if s == "auto" {
                        neori_ipc::OutputAction::Position {
                            position: neori_ipc::PositionToSet::Automatic,
                        }
                    } else {
                        return Err(LuaError::runtime(format!(
                            "set_position: expected a {{x, y}} table or \"auto\", got \"{s}\""
                        )));
                    }
                }
                mlua::Value::Table(t) => {
                    let x: i32 = t
                        .get("x")
                        .map_err(|e| LuaError::runtime(format!("set_position.x: {e}")))?;
                    let y: i32 = t
                        .get("y")
                        .map_err(|e| LuaError::runtime(format!("set_position.y: {e}")))?;
                    neori_ipc::OutputAction::Position {
                        position: neori_ipc::PositionToSet::Specific(
                            neori_ipc::ConfiguredPosition { x, y },
                        ),
                    }
                }
                other => {
                    return Err(LuaError::runtime(format!(
                        "set_position: expected a {{x, y}} table or \"auto\", got {}",
                        other.type_name()
                    )))
                }
            };
            enqueue_output_op(lua, &this.0.name, op);
            Ok(())
        });

        // out:set_vrr(true/false) — on-demand defaults to false
        methods.add_method(
            "set_vrr",
            |lua, this, args: (bool, Option<bool>)| {
                let (enable, on_demand) = args;
                enqueue_output_op(
                    lua,
                    &this.0.name,
                    neori_ipc::OutputAction::Vrr {
                        vrr: neori_ipc::VrrToSet {
                            vrr: enable,
                            on_demand: on_demand.unwrap_or(false),
                        },
                    },
                );
                Ok(())
            },
        );

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            let mode = this.0.mode.as_deref().unwrap_or("?");
            Ok(format!("Output({:?}, mode={mode})", this.0.name))
        });
    }
}

/// Enqueue an output configuration operation onto the shared pending-output-ops
/// queue stashed on the Lua state via [`Lua::set_app_data`].
fn enqueue_output_op(lua: &Lua, output_name: &str, op: neori_ipc::OutputAction) {
    if let Some(pending) = lua.app_data_ref::<PendingOutputOps>() {
        pending
            .lock()
            .unwrap()
            .push((output_name.to_string(), op));
    }
}

/// Parse a "WxH@REFRESH" mode string into a [`neori_ipc::ConfiguredMode`].
/// Accepts an omitted refresh rate (just "WxH" means Automatic refresh).
fn parse_mode_string_strict(s: &str) -> Result<neori_ipc::ConfiguredMode, String> {
    let (res, refresh) = match s.split_once('@') {
        Some((res, r)) => {
            let r: f64 = r
                .parse()
                .map_err(|_| format!("invalid refresh rate in mode {s:?}"))?;
            (res, Some(r))
        }
        None => (s, None),
    };
    let (w, h) = res
        .split_once('x')
        .ok_or_else(|| format!("mode {s:?} is missing an 'x' between width and height"))?;
    let width: u16 = w
        .parse()
        .map_err(|_| format!("invalid width in mode {s:?}"))?;
    let height: u16 = h
        .parse()
        .map_err(|_| format!("invalid height in mode {s:?}"))?;
    Ok(neori_ipc::ConfiguredMode {
        width,
        height,
        refresh,
    })
}

/// Parse a transform name into [`neori_ipc::Transform`]. Accepts both kebab-case
/// ("flipped-90") and camel-case variants, matching the strings the
/// `Output:transform()` accessor returns.
/// Lua `Workspace` userdata.
///
/// Same snapshot semantics as `Window` and `Output`: all read-only accessors
/// return the values captured at the moment the Workspace was built. Mutation
/// methods enqueue IPC `Action`s that target this workspace by its stable id.
#[derive(Debug, Clone)]
pub struct LuaWorkspace(pub WorkspaceSnapshot);

impl mlua::UserData for LuaWorkspace {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // Read-only accessors.
        methods.add_method("idx", |_, this, ()| Ok(this.0.idx));
        methods.add_method("id", |_, this, ()| Ok(this.0.id));
        methods.add_method("name", |_, this, ()| Ok(this.0.name.clone()));
        methods.add_method("output", |_, this, ()| Ok(this.0.output_name.clone()));
        methods.add_method("is_focused", |_, this, ()| Ok(this.0.is_focused));

        // ws:windows() returns the windows that live on this workspace, filtered
        // from the compositor-primed window cache.
        methods.add_method("windows", |lua, this, ()| {
            let ids: std::collections::HashSet<u64> =
                this.0.window_ids.iter().copied().collect();
            let arr = lua.create_table()?;
            if let Some(cache) = lua.app_data_ref::<SharedWindowCache>() {
                let locked = cache.lock().unwrap();
                let mut i = 1;
                for w in &locked.windows {
                    if ids.contains(&w.id) {
                        arr.set(i, Window(w.clone()))?;
                        i += 1;
                    }
                }
            }
            Ok(arr)
        });

        // Mutation methods — each enqueues an Action via the pending_actions queue.
        methods.add_method("focus", |lua, this, ()| {
            enqueue_action(
                lua,
                Action::FocusWorkspace {
                    reference: neori_ipc::WorkspaceReferenceArg::Id(this.0.id),
                },
            );
            Ok(())
        });

        methods.add_method("send_to_output", |lua, this, output: String| {
            enqueue_action(
                lua,
                Action::MoveWorkspaceToMonitor {
                    output,
                    reference: Some(neori_ipc::WorkspaceReferenceArg::Id(this.0.id)),
                },
            );
            Ok(())
        });

        methods.add_method("set_name", |lua, this, name: String| {
            enqueue_action(
                lua,
                Action::SetWorkspaceName {
                    name,
                    workspace: Some(neori_ipc::WorkspaceReferenceArg::Id(this.0.id)),
                },
            );
            Ok(())
        });

        methods.add_method("unset_name", |lua, this, ()| {
            enqueue_action(
                lua,
                Action::UnsetWorkspaceName {
                    reference: Some(neori_ipc::WorkspaceReferenceArg::Id(this.0.id)),
                },
            );
            Ok(())
        });

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            let name = this
                .0
                .name
                .as_deref()
                .map(|n| format!("{n:?}"))
                .unwrap_or_else(|| "nil".into());
            Ok(format!(
                "Workspace(idx={}, name={name}, id={})",
                this.0.idx, this.0.id
            ))
        });
    }
}

/// Lua `Device` userdata: a snapshot wrapper for an input device (keyboard /
/// mouse / touchpad / touch panel) with read-only metadata accessors and
/// mutation methods that enqueue libinput tuning ops keyed by the device's
/// udev `sysname`. The compositor drains the queue after the callback returns
/// and applies each op to the matching `input::Device`.
///
/// Named `LuaDevice` on the Rust side to avoid any `Device` ambiguity with
/// Smithay / libinput types; Lua users just see it as `Device`.
#[derive(Debug, Clone)]
pub struct LuaDevice(pub DeviceSnapshot);

impl mlua::UserData for LuaDevice {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // Read-only accessors.
        methods.add_method("name", |_, this, ()| Ok(this.0.name.clone()));
        methods.add_method("sysname", |_, this, ()| Ok(this.0.sysname.clone()));
        methods.add_method("product_id", |_, this, ()| Ok(this.0.product_id));
        methods.add_method("vendor_id", |_, this, ()| Ok(this.0.vendor_id));
        methods.add_method("is_pointer", |_, this, ()| Ok(this.0.is_pointer));
        methods.add_method("is_keyboard", |_, this, ()| Ok(this.0.is_keyboard));
        methods.add_method("is_touch", |_, this, ()| Ok(this.0.is_touch));
        methods.add_method("is_touchpad", |_, this, ()| Ok(this.0.is_touchpad));

        // Mutation methods. Each enqueues a (sysname, DeviceOp) pair; the
        // compositor drains and applies via `input::Device::config_*_set_*`.
        // Changes persist until the next config reload wipes them.
        methods.add_method("set_tap", |lua, this, enable: bool| {
            enqueue_device_op(lua, &this.0.sysname, DeviceOp::SetTap(enable));
            Ok(())
        });
        methods.add_method("set_natural_scroll", |lua, this, enable: bool| {
            enqueue_device_op(lua, &this.0.sysname, DeviceOp::SetNaturalScroll(enable));
            Ok(())
        });
        methods.add_method("set_accel", |lua, this, speed: f64| {
            if !(-1.0..=1.0).contains(&speed) {
                return Err(LuaError::runtime(format!(
                    "Device:set_accel: speed {speed} out of range [-1.0, 1.0]"
                )));
            }
            enqueue_device_op(lua, &this.0.sysname, DeviceOp::SetAccel(speed));
            Ok(())
        });
        methods.add_method("set_left_handed", |lua, this, enable: bool| {
            enqueue_device_op(lua, &this.0.sysname, DeviceOp::SetLeftHanded(enable));
            Ok(())
        });
        methods.add_method("set_disabled", |lua, this, disabled: bool| {
            enqueue_device_op(lua, &this.0.sysname, DeviceOp::SetDisabled(disabled));
            Ok(())
        });

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "Device(sysname={:?}, name={:?})",
                this.0.sysname, this.0.name
            ))
        });
    }
}

fn enqueue_device_op(lua: &Lua, sysname: &str, op: DeviceOp) {
    if let Some(queue) = lua.app_data_ref::<PendingDeviceOps>() {
        queue.lock().unwrap().push((sysname.to_owned(), op));
    }
}

fn parse_transform_string(s: &str) -> Result<neori_ipc::Transform, String> {
    match s {
        "normal" => Ok(neori_ipc::Transform::Normal),
        "90" => Ok(neori_ipc::Transform::_90),
        "180" => Ok(neori_ipc::Transform::_180),
        "270" => Ok(neori_ipc::Transform::_270),
        "flipped" => Ok(neori_ipc::Transform::Flipped),
        "flipped-90" | "flipped_90" => Ok(neori_ipc::Transform::Flipped90),
        "flipped-180" | "flipped_180" => Ok(neori_ipc::Transform::Flipped180),
        "flipped-270" | "flipped_270" => Ok(neori_ipc::Transform::Flipped270),
        other => Err(format!(
            "invalid transform {other:?}: expected one of \
             \"normal\", \"90\", \"180\", \"270\", \"flipped\", \
             \"flipped-90\", \"flipped-180\", \"flipped-270\""
        )),
    }
}

/// Lua `Window` userdata: a snapshot wrapper with read-only accessors.
#[derive(Debug, Clone)]
pub struct Window(pub WindowSnapshot);

impl mlua::UserData for Window {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // Read-only accessors.
        methods.add_method("id", |_, this, ()| Ok(this.0.id));
        methods.add_method("app_id", |_, this, ()| Ok(this.0.app_id.clone()));
        methods.add_method("title", |_, this, ()| Ok(this.0.title.clone()));
        methods.add_method("is_focused", |_, this, ()| Ok(this.0.is_focused));
        methods.add_method("is_floating", |_, this, ()| Ok(this.0.is_floating));
        methods.add_method("is_fullscreen", |_, this, ()| Ok(this.0.is_fullscreen));
        methods.add_method("workspace", |lua, this, ()| {
            if this.0.workspace_idx.is_none() && this.0.workspace_name.is_none() {
                return Ok(mlua::Value::Nil);
            }
            let t = lua.create_table()?;
            if let Some(idx) = this.0.workspace_idx {
                t.set("idx", idx)?;
            }
            if let Some(name) = &this.0.workspace_name {
                t.set("name", name.clone())?;
            }
            Ok(mlua::Value::Table(t))
        });
        methods.add_method("output", |_, this, ()| Ok(this.0.output_name.clone()));

        // Mutation methods. Each enqueues the matching IPC Action against this
        // window's stable id; the compositor drains the queue after the callback
        // returns. Stale ids (window already gone) are silently ignored by the
        // compositor's dispatch, so it's safe to call these on a Window value
        // stashed in a Lua local across callbacks.

        methods.add_method("close", |lua, this, ()| {
            enqueue_action(lua, Action::CloseWindow { id: Some(this.0.id) });
            Ok(())
        });

        methods.add_method("focus", |lua, this, ()| {
            enqueue_action(lua, Action::FocusWindow { id: this.0.id });
            Ok(())
        });

        // `w:fullscreen(true)` / `w:fullscreen(false)` / `w:fullscreen()` (toggle).
        // Set-style calls skip the IPC toggle when the snapshot already matches the
        // requested state, so they're idempotent in the common case. If the snapshot
        // was stale (the user stashed `w` across callbacks and state changed), a
        // mismatched toggle can still land — re-query via neori.focused_window() /
        // neori.windows() when you need the current state.
        methods.add_method(
            "fullscreen",
            |lua, this, enable: Option<bool>| {
                if let Some(desired) = enable {
                    if desired == this.0.is_fullscreen {
                        return Ok(());
                    }
                }
                enqueue_action(lua, Action::FullscreenWindow { id: Some(this.0.id) });
                Ok(())
            },
        );

        // `w:float(true)` / `w:float(false)` / `w:float()` (toggle). The explicit
        // MoveWindowToFloating / MoveWindowToTiling IPC actions make this truly
        // idempotent regardless of snapshot staleness.
        methods.add_method("float", |lua, this, enable: Option<bool>| {
            let desired = enable.unwrap_or(!this.0.is_floating);
            if desired == this.0.is_floating {
                return Ok(());
            }
            let action = if desired {
                Action::MoveWindowToFloating { id: Some(this.0.id) }
            } else {
                Action::MoveWindowToTiling { id: Some(this.0.id) }
            };
            enqueue_action(lua, action);
            Ok(())
        });

        // `w:send_to_workspace(2)` / `w:send_to_workspace("browser")`.
        methods.add_method(
            "send_to_workspace",
            |lua, this, target: mlua::Value| {
                let reference = workspace_reference_from_lua(&target)?;
                enqueue_action(
                    lua,
                    Action::MoveWindowToWorkspace {
                        window_id: Some(this.0.id),
                        reference,
                        focus: true,
                    },
                );
                Ok(())
            },
        );

        // `w:send_to_output("eDP-1")`.
        methods.add_method(
            "send_to_output",
            |lua, this, output: String| {
                enqueue_action(
                    lua,
                    Action::MoveWindowToMonitor {
                        id: Some(this.0.id),
                        output,
                    },
                );
                Ok(())
            },
        );

        // Pre-placement overrides. These only have an effect from inside a
        // `window-opening` handler: the compositor sets the overrides guard
        // before firing the event, these methods write into it, and the
        // compositor drains + merges into the per-window ResolvedWindowRules
        // right before placement. Outside the event (e.g. called on a stashed
        // Window after the fact), the guard is `None`, so the write is a silent
        // no-op — nothing leaks onto some later window.

        methods.add_method("open_on_output", |lua, _, name: String| {
            with_open_overrides(lua, |o| o.open_on_output = Some(name));
            Ok(())
        });

        methods.add_method("open_on_workspace", |lua, _, name: String| {
            with_open_overrides(lua, |o| o.open_on_workspace = Some(name));
            Ok(())
        });

        methods.add_method("open_fullscreen", |lua, _, enable: Option<bool>| {
            let v = enable.unwrap_or(true);
            with_open_overrides(lua, |o| o.open_fullscreen = Some(v));
            Ok(())
        });

        methods.add_method("open_maximized", |lua, _, enable: Option<bool>| {
            let v = enable.unwrap_or(true);
            with_open_overrides(lua, |o| o.open_maximized = Some(v));
            Ok(())
        });

        methods.add_method("open_floating", |lua, _, enable: Option<bool>| {
            let v = enable.unwrap_or(true);
            with_open_overrides(lua, |o| o.open_floating = Some(v));
            Ok(())
        });

        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            let app = this.0.app_id.as_deref().unwrap_or("?");
            let title = this.0.title.as_deref().unwrap_or("?");
            Ok(format!("Window(#{}, app_id={app:?}, title={title:?})", this.0.id))
        });
    }
}

/// Enqueue an [`Action`] onto the shared pending-actions queue stashed on the Lua
/// state via [`Lua::set_app_data`].
///
/// A missing app_data is a no-op; it only happens if someone constructs a `Window`
/// userdata on a Lua state that isn't owned by a [`ScriptHost`], which we don't do
/// outside of unit tests. In that case the mutation silently drops rather than
/// erroring from Lua.
fn enqueue_action(lua: &Lua, action: Action) {
    if let Some(pending) = lua.app_data_ref::<PendingActions>() {
        pending.lock().unwrap().push(action);
    }
}

/// Mutate the open-overrides accumulator, if one is in scope. Outside a
/// `window-opening` dispatch the inner `Option` is `None` and the write is
/// silently dropped — see [`OpenOverrides`].
fn with_open_overrides(lua: &Lua, f: impl FnOnce(&mut OpenOverrides)) {
    if let Some(guard) = lua.app_data_ref::<SharedOpenOverrides>() {
        if let Some(overrides) = guard.lock().unwrap().as_mut() {
            f(overrides);
        }
    }
}

/// Convert a Lua value into a [`neori_ipc::WorkspaceReferenceArg`].
///
/// Accepts an integer (1-based global workspace index) or a string (workspace name).
fn workspace_reference_from_lua(
    value: &mlua::Value,
) -> mlua::Result<neori_ipc::WorkspaceReferenceArg> {
    match value {
        mlua::Value::Integer(i) => {
            let idx = u8::try_from(*i).map_err(|_| {
                LuaError::runtime(format!(
                    "workspace index {i} out of range (expected 0..=255)"
                ))
            })?;
            Ok(neori_ipc::WorkspaceReferenceArg::Index(idx))
        }
        mlua::Value::Number(n) => {
            let n = *n;
            if n.fract() != 0.0 || n < 0.0 || n > u8::MAX as f64 {
                return Err(LuaError::runtime(format!(
                    "workspace index {n} is not a non-negative integer that fits in u8"
                )));
            }
            Ok(neori_ipc::WorkspaceReferenceArg::Index(n as u8))
        }
        mlua::Value::String(s) => {
            let s = s.to_str().map_err(|e| LuaError::runtime(format!("{e}")))?;
            Ok(neori_ipc::WorkspaceReferenceArg::Name(s.to_string()))
        }
        other => Err(LuaError::runtime(format!(
            "workspace reference must be an integer or a string, got {}",
            other.type_name()
        ))),
    }
}

/// A persistent Lua host for the compositor.
///
/// Owns a Lua state with the neori config API and keeps the function callbacks registered
/// through `neori.on(event, function)` and `neori.bind(key, function)`. The compositor
/// keeps this alive so that events and keybinds can be dispatched to Lua callbacks at
/// runtime.
pub struct ScriptHost {
    lua: Lua,
    event_functions: EventRegistry,
    bind_functions: BindFnRegistry,
    /// Monotonic id counter for [`Action::ScriptFunction`] references.
    next_bind_fn_id: Arc<AtomicU64>,
    /// Queue of actions enqueued by Lua via `neori.action`.
    pending_actions: PendingActions,
    /// Queue of output configuration operations enqueued by `Output` mutation
    /// methods (`out:set_mode(…)`, `out:set_scale(…)`, …).
    pending_output_ops: PendingOutputOps,
    /// Cached compositor state powering `neori.windows()` and
    /// `neori.focused_window()`. The compositor refreshes it before every Lua call.
    window_cache: SharedWindowCache,
    /// Cached compositor state powering `neori.outputs()` and
    /// `neori.focused_output()`.
    output_cache: SharedOutputCache,
    /// Cached compositor state powering `neori.workspaces()` and
    /// `neori.focused_workspace()`.
    workspace_cache: SharedWorkspaceCache,
    /// Pre-placement overrides for `window-opening` handlers. `Some(default)`
    /// only during a `window-opening` dispatch; `None` the rest of the time.
    /// `Window` userdata `open_*` methods write to the inner struct when the
    /// guard is `Some`, and silently no-op when it's `None`.
    open_overrides: SharedOpenOverrides,
    /// Current pointer position, refreshed by the compositor before every
    /// callback. Powers `neori.pointer()`.
    pointer_cache: SharedPointerCache,
    /// Cursor warps enqueued by Lua via `neori.warp(x, y)`. The compositor
    /// drains after the callback returns.
    pending_warps: PendingWarps,
    /// Current keyboard layout name, refreshed by the compositor. Powers
    /// `neori.keyboard_layout()`.
    keyboard_layout: SharedKeyboardLayout,
    /// Snapshot of connected input devices, refreshed by the compositor.
    /// Powers `neori.devices()`.
    device_cache: SharedDeviceCache,
    /// Pending libinput mutations enqueued by `Device:set_*` methods.
    pending_device_ops: PendingDeviceOps,
    /// Sink for Lua `print(...)` output. When `None`, prints go to stdout.
    /// The `print` override installed in [`ScriptHost::new`] reads this slot on
    /// every call, so [`set_print_log`](Self::set_print_log) can swap it in at
    /// any time (including after a config reload).
    print_sink: PrintSinkSlot,
}

impl ScriptHost {
    pub fn new() -> Result<Self, String> {
        let lua = Lua::new();
        let event_functions = Arc::new(Mutex::new(HashMap::new()));
        let bind_functions = Arc::new(Mutex::new(HashMap::new()));
        let next_bind_fn_id = Arc::new(AtomicU64::new(1));
        let pending_actions: PendingActions = Arc::new(Mutex::new(Vec::new()));
        let pending_output_ops: PendingOutputOps = Arc::new(Mutex::new(Vec::new()));
        let window_cache: SharedWindowCache = Arc::new(Mutex::new(WindowCache::default()));
        let output_cache = Arc::new(Mutex::new(OutputCache::default()));
        let workspace_cache = Arc::new(Mutex::new(WorkspaceCache::default()));
        let open_overrides: SharedOpenOverrides = Arc::new(Mutex::new(None));
        let pointer_cache: SharedPointerCache = Arc::new(Mutex::new(None));
        let pending_warps: PendingWarps = Arc::new(Mutex::new(Vec::new()));
        let keyboard_layout: SharedKeyboardLayout = Arc::new(Mutex::new(None));
        let device_cache: SharedDeviceCache = Arc::new(Mutex::new(DeviceCache::default()));
        let pending_device_ops: PendingDeviceOps = Arc::new(Mutex::new(Vec::new()));
        let print_sink: PrintSinkSlot = Arc::new(Mutex::new(None));

        // Stash handles for every runtime queue + cache on the Lua state so userdata
        // methods (`w:close()`, `out:set_scale(…)`, `ws:windows()`, …) can reach
        // them via `lua.app_data_ref` without taking a reference to Self.
        lua.set_app_data::<PendingActions>(Arc::clone(&pending_actions));
        lua.set_app_data::<PendingOutputOps>(Arc::clone(&pending_output_ops));
        lua.set_app_data::<SharedWindowCache>(Arc::clone(&window_cache));
        lua.set_app_data::<SharedOpenOverrides>(Arc::clone(&open_overrides));
        lua.set_app_data::<PendingWarps>(Arc::clone(&pending_warps));
        lua.set_app_data::<PendingDeviceOps>(Arc::clone(&pending_device_ops));

        // Override Lua's default `print` so it routes to the installed sink
        // (set later via `set_print_log`) or falls back to stdout. Formats each
        // line with a UTC timestamp prefix so logs are post-mortem-friendly.
        {
            let sink_slot = Arc::clone(&print_sink);
            let func = lua
                .create_function(move |lua, args: mlua::Variadic<Value>| {
                    let tostring: mlua::Function = lua.globals().get("tostring")?;
                    let mut body = String::new();
                    for (i, v) in args.into_iter().enumerate() {
                        if i > 0 {
                            body.push('\t');
                        }
                        let s: mlua::String = tostring.call(v)?;
                        body.push_str(&s.to_string_lossy());
                    }

                    let ts = format_utc_timestamp(SystemTime::now());
                    let line = format!("[{ts}] {body}\n");

                    let sink = sink_slot.lock().unwrap().clone();
                    if let Some(sink) = sink {
                        let mut w = sink.lock().unwrap();
                        let _ = w.write_all(line.as_bytes());
                        let _ = w.flush();
                    } else {
                        let _ = std::io::stdout().write_all(line.as_bytes());
                    }
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
            lua.globals()
                .set("print", func)
                .map_err(|e| e.to_string())?;
        }

        Ok(Self {
            lua,
            event_functions,
            bind_functions,
            next_bind_fn_id,
            pending_actions,
            pending_output_ops,
            window_cache,
            output_cache,
            workspace_cache,
            open_overrides,
            pointer_cache,
            pending_warps,
            keyboard_layout,
            device_cache,
            pending_device_ops,
            print_sink,
        })
    }

    /// Install a sink for Lua `print(...)` output. Overwrites any previously
    /// installed sink; pass the same [`PrintSink`] into each new host across
    /// config reloads to keep the underlying file open.
    pub fn set_print_log(&self, sink: PrintSink) {
        *self.print_sink.lock().unwrap() = Some(sink);
    }

    /// Replace the cached device snapshots.
    pub fn set_device_cache(&self, cache: DeviceCache) {
        *self.device_cache.lock().unwrap() = cache;
    }

    pub fn clear_device_cache(&self) {
        *self.device_cache.lock().unwrap() = DeviceCache::default();
    }

    /// Clear every Lua-facing cache (window / output / workspace / pointer /
    /// device) in one call.
    ///
    /// The compositor calls this after each callback dispatch so stale snapshots
    /// from the priming step don't leak into unrelated accessor paths taken
    /// later in the same tick. Adding a new cache in the future only needs this
    /// one function updated — every fire site goes through here.
    pub fn clear_script_caches(&self) {
        self.clear_window_cache();
        self.clear_output_cache();
        self.clear_workspace_cache();
        self.clear_pointer_cache();
        self.clear_device_cache();
    }

    /// Drain pending libinput device mutation ops enqueued by `Device:set_*`.
    pub fn drain_pending_device_ops(&self) -> Vec<(String, DeviceOp)> {
        std::mem::take(&mut *self.pending_device_ops.lock().unwrap())
    }

    /// Replace the cached pointer snapshot, called by the compositor just before
    /// each Lua callback dispatch.
    pub fn set_pointer_cache(&self, snapshot: PointerSnapshot) {
        *self.pointer_cache.lock().unwrap() = Some(snapshot);
    }

    pub fn clear_pointer_cache(&self) {
        *self.pointer_cache.lock().unwrap() = None;
    }

    /// Drain pending cursor warps enqueued by Lua via `neori.warp(x, y)`.
    pub fn drain_pending_warps(&self) -> Vec<(f64, f64)> {
        std::mem::take(&mut *self.pending_warps.lock().unwrap())
    }

    /// Replace the cached keyboard layout name.
    pub fn set_keyboard_layout(&self, name: Option<String>) {
        *self.keyboard_layout.lock().unwrap() = name;
    }

    pub fn clear_keyboard_layout(&self) {
        *self.keyboard_layout.lock().unwrap() = None;
    }

    /// Open a `window-opening` dispatch: enables the `w:open_*` methods and
    /// clears any stale accumulator from a prior dispatch. The compositor calls
    /// this just before firing the `window-opening` event, and
    /// [`ScriptHost::take_open_overrides`] right after.
    pub fn begin_open_overrides(&self) {
        *self.open_overrides.lock().unwrap() = Some(OpenOverrides::default());
    }

    /// Take the overrides accumulated during a `window-opening` dispatch and
    /// reset the guard to `None`. A later stray `w:open_*` call from a stashed
    /// `Window` is a no-op until the next [`ScriptHost::begin_open_overrides`].
    pub fn take_open_overrides(&self) -> OpenOverrides {
        self.open_overrides
            .lock()
            .unwrap()
            .take()
            .unwrap_or_default()
    }

    /// Replace the current output cache with a fresh snapshot.
    pub fn set_output_cache(&self, cache: OutputCache) {
        *self.output_cache.lock().unwrap() = cache;
    }

    pub fn clear_output_cache(&self) {
        *self.output_cache.lock().unwrap() = OutputCache::default();
    }

    pub fn set_workspace_cache(&self, cache: WorkspaceCache) {
        *self.workspace_cache.lock().unwrap() = cache;
    }

    pub fn clear_workspace_cache(&self) {
        *self.workspace_cache.lock().unwrap() = WorkspaceCache::default();
    }

    /// Drain and return any pending output configuration ops enqueued via the
    /// `Output` userdata mutation methods.
    pub fn drain_pending_output_ops(&self) -> Vec<(String, neori_ipc::OutputAction)> {
        let mut queue = self.pending_output_ops.lock().unwrap();
        std::mem::take(&mut *queue)
    }

    /// Replaces the current window cache with a fresh snapshot.
    ///
    /// The compositor calls this right before every Lua callback invocation so
    /// `neori.windows()` and `neori.focused_window()` see current compositor state.
    pub fn set_window_cache(&self, cache: WindowCache) {
        *self.window_cache.lock().unwrap() = cache;
    }

    /// Clears the window cache. Called after a Lua callback returns so stale state does
    /// not leak into a later direct-API call that bypasses the cache refresh.
    pub fn clear_window_cache(&self) {
        *self.window_cache.lock().unwrap() = WindowCache::default();
    }

    /// Runs the config script at `path` in this host.
    ///
    /// Resets all previously registered event callbacks, then evaluates the script and
    /// parses the resulting config. Returns the parsed config together with the list of
    /// included files for the config watcher.
    pub fn run(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<(Config, Vec<PathBuf>), ParseLuaError> {
        let includes: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let parse_err = |message: String| {
            let includes = includes.lock().unwrap().clone();
            ParseLuaError { message, includes }
        };
        self.event_functions.lock().unwrap().clear();
        self.bind_functions.lock().unwrap().clear();
        self.pending_actions.lock().unwrap().clear();
        self.pending_output_ops.lock().unwrap().clear();
        *self.window_cache.lock().unwrap() = WindowCache::default();
        *self.output_cache.lock().unwrap() = OutputCache::default();
        *self.workspace_cache.lock().unwrap() = WorkspaceCache::default();
        *self.pointer_cache.lock().unwrap() = None;
        self.pending_warps.lock().unwrap().clear();
        *self.keyboard_layout.lock().unwrap() = None;
        *self.device_cache.lock().unwrap() = DeviceCache::default();
        self.pending_device_ops.lock().unwrap().clear();

        set_strict_config(matches!(
            std::env::var("NEORI_STRICT_CONFIG").ok().as_deref(),
            Some("1") | Some("true") | Some("yes") | Some("on")
        ));

        let package = self
            .lua
            .globals()
            .get::<LuaTable>("package")
            .map_err(|e| parse_err(e.to_string()))?;
        let path_str = package
            .get::<LuaString>("path")
            .map_err(|e| parse_err(e.to_string()))?
            .to_str()
            .map_err(|e| parse_err(e.to_string()))?
            .to_string();
        if let Some(parent) = path.parent() {
            let lua_path = format!(
                "{}/?.lua;{}/?/init.lua;{}",
                parent.display(),
                parent.display(),
                path_str
            );
            package
                .set("path", lua_path)
                .map_err(|e| parse_err(e.to_string()))?;
        }

        let search_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("/"))
            .to_path_buf();

        register_require_searcher(&self.lua, search_dir.clone(), &includes).map_err(&parse_err)?;

        let api_data = setup_neori_api(
            &self.lua,
            &self.event_functions,
            &self.bind_functions,
            &self.next_bind_fn_id,
            &self.pending_actions,
            &self.window_cache,
            &self.output_cache,
            &self.workspace_cache,
            &self.pointer_cache,
            &self.pending_warps,
            &self.keyboard_layout,
            &self.device_cache,
            search_dir.clone(),
            Arc::clone(&includes),
        )
        .map_err(&parse_err)?;

        let chunk = self
            .lua
            .load(text)
            .set_name(path.to_string_lossy().to_string());

        let result: Value = chunk
            .eval()
            .map_err(|e| parse_err(format!("Lua error: {e}")))?;

        let table = match result {
            Value::Table(t) => {
                merge_api_data(&t, &api_data).map_err(&parse_err)?;
                t
            }
            Value::Nil => api_data,
            _ => {
                return Err(parse_err(format!("expected table or nil, got {result:?}")));
            }
        };

        let mut config = Config::default();
        PARSE_ENV.with(|env| *env.borrow_mut() = Some((search_dir, Arc::clone(&includes))));
        let res = parse_table_into_config(table, &mut config);
        PARSE_ENV.with(|env| *env.borrow_mut() = None);
        res.map_err(parse_err)?;
        let includes = includes.lock().unwrap().clone();
        Ok((config, includes))
    }

    /// Calls every Lua function registered for `event`, in registration order.
    ///
    /// Returns any Actions the callbacks enqueued through `neori.action`. Static action
    /// handlers for the same event live in `Config` and are dispatched separately by the
    /// compositor.
    pub fn call_event(&self, event: &str) -> Result<Vec<Action>, String> {
        self.call_event_with(event, ())
    }

    /// Like [`ScriptHost::call_event`], but passes `args` to every callback.
    pub fn call_event_with<A>(&self, event: &str, args: A) -> Result<Vec<Action>, String>
    where
        A: IntoLuaMulti + Clone,
    {
        let funs = {
            let registry = self.event_functions.lock().unwrap();
            match registry.get(event) {
                Some(fs) => fs.clone(),
                None => return Ok(Vec::new()),
            }
        };
        // Run every handler, capturing only the first error. One bad
        // subscriber must not disable the others — this matches typical
        // pub/sub semantics and keeps a bug in one handler from silently
        // breaking the rest of the config.
        let mut first_error: Option<String> = None;
        for f in &funs {
            if let Err(e) = f.call::<()>(args.clone()) {
                first_error.get_or_insert_with(|| format!("{event} callback error: {e}"));
            }
        }
        // Always drain pending_actions, even on error, so an Action a
        // surviving handler queued doesn't leak into the next event's
        // dispatch — where it would run with the wrong event's context.
        let actions = self.drain_pending_actions();
        match first_error {
            Some(e) => {
                // Current Result signature can't surface both; the compositor
                // treats Err as "flash the config-error notification, run
                // nothing." Actions from surviving handlers are dropped on
                // this path, same as the single-handler error case before
                // this change. The important fix is that siblings still ran
                // and nothing leaks.
                let _ = actions;
                Err(e)
            }
            None => Ok(actions),
        }
    }

    /// Reads a global value from the Lua state, for introspection in tests.
    pub fn read_global(&self, key: &str) -> mlua::Result<Value> {
        self.lua.globals().get::<Value>(key)
    }

    /// Calls the Lua function registered for `id`, in registration order if multiple
    /// callbacks share the same id (currently each bind has a unique id).
    ///
    /// Returns the list of Actions that the function enqueued through `neori.action`.
    /// The compositor should then dispatch them.
    pub fn call_bind_function(&self, id: u64) -> Result<Vec<Action>, String> {
        let fun = {
            let funs = self.bind_functions.lock().unwrap();
            match funs.get(&id) {
                Some(f) => f.clone(),
                None => return Err(format!("no Lua function registered for bind id {id}")),
            }
        };
        fun.call::<()>(())
            .map_err(|e| format!("bind callback error: {e}"))?;
        Ok(self.drain_pending_actions())
    }

    /// Drains and returns any [`Action`]s that Lua code enqueued via `neori.action`.
    pub fn drain_pending_actions(&self) -> Vec<Action> {
        let mut queue = self.pending_actions.lock().unwrap();
        std::mem::take(&mut *queue)
    }

    /// Returns whether a bind function is registered for `id`.
    pub fn has_bind_function(&self, id: u64) -> bool {
        self.bind_functions.lock().unwrap().contains_key(&id)
    }

    /// Returns whether at least one Lua handler is registered for `event`. Lets
    /// the compositor skip cache priming / snapshot building when no one would
    /// consume the fire.
    pub fn has_event_handlers(&self, event: &str) -> bool {
        self.event_functions
            .lock()
            .unwrap()
            .get(event)
            .is_some_and(|fs| !fs.is_empty())
    }
}

impl Default for ScriptHost {
    fn default() -> Self {
        // Creating a Lua state cannot realistically fail; fall back to a fresh host.
        Self::new().expect("error creating script host")
    }
}

/// Format a [`SystemTime`] as `YYYY-MM-DD HH:MM:SS.mmm` UTC, no dependencies.
///
/// Uses the Hinnant civil-from-days algorithm
/// (<https://howardhinnant.github.io/date_algorithms.html>). Only used by the
/// `print` override's timestamp prefix, so precision is capped at milliseconds
/// and timezone is fixed to UTC.
fn format_utc_timestamp(now: SystemTime) -> String {
    let d = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400) as u32;
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d_m = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }

    format!("{y:04}-{m:02}-{d_m:02} {hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

pub fn parse_lua(path: &Path, text: &str) -> Result<ParsedConfig, ParseLuaError> {
    parse_lua_with_host(path, text)
}

/// Like [`parse_lua`] but returns the [`ScriptHost`] alongside the parsed config, so the
/// caller (e.g. the compositor) can keep firing events at the registered Lua callbacks.
pub fn parse_lua_with_host(path: &Path, text: &str) -> Result<ParsedConfig, ParseLuaError> {
    let mut host = ScriptHost::new().map_err(|message| ParseLuaError {
        message,
        includes: Vec::new(),
    })?;
    let (config, includes) = host.run(path, text)?;
    Ok(ParsedConfig {
        config,
        includes,
        host,
    })
}

/// Installs a custom `package.searchers` entry so that Lua's `require` resolves modules
/// relative to the config's directory and records every included file for the config
/// watcher. Includes are recorded before they are evaluated, so a broken included file
/// still gets watched.
fn register_require_searcher(
    lua: &Lua,
    search_dir: PathBuf,
    includes: &Arc<Mutex<Vec<PathBuf>>>,
) -> Result<(), String> {
    let err = |e: LuaError| format!("{e}");

    let searchers = lua
        .globals()
        .get::<LuaTable>("package")
        .map_err(err)?
        .get::<LuaTable>("searchers")
        .map_err(err)?;

    let includes = Arc::clone(includes);
    let searcher = lua
        .create_function(move |lua, module_name: String| {
            let dotted = module_name.replace('.', "/");
            let candidates = [
                search_dir.join(format!("{dotted}.lua")),
                search_dir.join(&dotted).join("init.lua"),
            ];

            for candidate in candidates {
                if !candidate.exists() {
                    continue;
                }
                let content = std::fs::read_to_string(&candidate).map_err(|e| {
                    LuaError::external(format!("error reading {:?}: {e}", candidate))
                })?;
                {
                    let mut includes = includes.lock().unwrap();
                    if !includes.contains(&candidate) {
                        includes.push(candidate.clone());
                    }
                }
                let loader = lua
                    .create_function(move |lua, _: ()| -> LuaResult<LuaValue> {
                        lua.load(&content)
                            .set_name(candidate.display().to_string())
                            .exec()?;
                        let neori = lua.globals().get::<LuaValue>("neori")?;
                        Ok(neori)
                    })
                    .map_err(|e| LuaError::external(e.to_string()))?;
                return Ok(loader);
            }

            Err(LuaError::external(format!(
                "neori: module '{module_name}' not found in {}",
                search_dir.display()
            )))
        })
        .map_err(err)?;

    searchers.set(2, searcher).map_err(err)?;
    Ok(())
}

/// Resolves an `neori.include` pattern into a sorted list of files to evaluate.
///
/// The pattern is resolved relative to `search_dir` unless absolute; a leading `~/` expands to
/// `$HOME`. A directory yields every `*.lua` file inside it, a plain file yields itself, and a
/// `*`/`?` glob in the final component is expanded against its parent directory.
fn expand_include_pattern(search_dir: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let expanded = if let Some(rest) = pattern.strip_prefix("~/") {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(pattern)
    };
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        search_dir.join(expanded)
    };

    if abs.is_file() {
        return Ok(vec![abs]);
    }
    if abs.is_dir() {
        return list_lua_files(&abs);
    }

    let dir = abs.parent().unwrap_or(Path::new("."));
    let file_pat = abs.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !file_pat.contains('*') && !file_pat.contains('?') {
        return Err(format!("neori.include: {} not found", abs.display()));
    }
    let re = glob_to_regex(file_pat)?;
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if re.is_match(name) {
                        files.push(path);
                    }
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Returns every `*.lua` file directly inside `dir`, sorted by path.
fn list_lua_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("error reading {}: {e}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("lua") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Compiles a simple glob (`*`, `?`) into a regular expression matching whole file names.
fn glob_to_regex(pattern: &str) -> Result<regex::Regex, String> {
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| format!("invalid include glob '{pattern}': {e}"))
}

/// Creates the `neori` global table with the config API functions.
///
/// The functions write into a separate `data` table that has the same shape as the
/// `return { ... }` table style. After the script runs, `data` is merged with the return
/// value (see [`merge_api_data`]) and parsed as usual. Returns the `data` table.
#[allow(clippy::too_many_arguments)]
fn setup_neori_api(
    lua: &Lua,
    event_functions: &EventRegistry,
    bind_functions: &BindFnRegistry,
    next_bind_fn_id: &Arc<AtomicU64>,
    pending_actions: &PendingActions,
    window_cache: &SharedWindowCache,
    output_cache: &SharedOutputCache,
    workspace_cache: &SharedWorkspaceCache,
    pointer_cache: &SharedPointerCache,
    pending_warps: &PendingWarps,
    keyboard_layout: &SharedKeyboardLayout,
    device_cache: &SharedDeviceCache,
    search_dir: PathBuf,
    includes: Arc<Mutex<Vec<PathBuf>>>,
) -> Result<LuaTable, String> {
    let err = |e: LuaError| format!("{e}");
    let data = lua.create_table().map_err(err)?;
    let neori = lua.create_table().map_err(err)?;

    // neori.set(key, value) assigns any top-level config key directly.
    {
        let data = data.clone();
        let func = lua
            .create_function(move |_, (key, value): (String, Value)| {
                data.set(key, value)?;
                Ok(())
            })
            .map_err(err)?;
        neori.set("set", func).map_err(err)?;
    }

    // neori.strict(on) toggles strict parsing, where unknown config keys become errors
    // instead of warnings. Called with no argument it enables strict mode.
    {
        let func = lua
            .create_function(move |_, args: mlua::Variadic<Value>| {
                let on = match args.first() {
                    Some(v) => value_is_truthy(v),
                    None => true,
                };
                set_strict_config(on);
                Ok(())
            })
            .map_err(err)?;
        neori.set("strict", func).map_err(err)?;
    }

    // neori.include(path, ...) evaluates other Lua files in the same environment.
    //
    // Each path is resolved relative to the config's directory unless absolute, and `~/` is
    // expanded. A directory loads every `*.lua` file inside it (sorted), and a `*`/`?` glob in
    // the final path component is supported. Every loaded file is recorded for the config
    // watcher, and files already loaded are skipped.
    {
        let includes = Arc::clone(&includes);
        let search_dir = search_dir.clone();
        let func = lua
            .create_function(move |lua, patterns: mlua::Variadic<String>| {
                for pattern in patterns {
                    let files = expand_include_pattern(&search_dir, &pattern)
                        .map_err(LuaError::external)?;
                    if files.is_empty() {
                        return Err(LuaError::external(format!(
                            "neori.include: no files matched '{pattern}'"
                        )));
                    }
                    for file in files {
                        {
                            let mut inc = includes.lock().unwrap();
                            if inc.contains(&file) {
                                continue;
                            }
                            inc.push(file.clone());
                        }
                        let content = std::fs::read_to_string(&file).map_err(|e| {
                            LuaError::external(format!("error reading {}: {e}", file.display()))
                        })?;
                        lua.load(&content)
                            .set_name(file.display().to_string())
                            .exec()?;
                    }
                }
                Ok(())
            })
            .map_err(err)?;
        neori.set("include", func).map_err(err)?;
    }

    // neori.bind(key, action) accepts a string, a table, or a Lua function. Binds are
    // appended to a sequence so that definition order is preserved (it matters for the
    // hotkey overlay). A function callback is stored in `bind_functions` keyed by a
    // monotonic id; the parsed Bind carries that id in `Action::ScriptFunction`.
    {
        let data = data.clone();
        let bind_functions = Arc::clone(bind_functions);
        let next_bind_fn_id = Arc::clone(next_bind_fn_id);
        let func = lua
            .create_function(move |lua, (key, action): (String, Value)| {
                match &action {
                    Value::String(_) | Value::Table(_) | Value::Function(_) => {}
                    _ => {
                        return Err(LuaError::runtime(
                            "action must be a string, a table, or a function",
                        ))
                    }
                }
                key.parse::<Key>()
                    .map_err(|e| LuaError::runtime(format!("invalid keybind '{key}': {e}")))?;

                let binds: Option<LuaTable> = data.get("binds")?;
                let binds = match binds {
                    Some(binds) => binds,
                    None => {
                        let binds = lua.create_table()?;
                        data.set("binds", binds.clone())?;
                        binds
                    }
                };
                for pair in binds.clone().pairs::<Value, Value>() {
                    let (existing_key, existing_value) = pair?;
                    let existing = match &existing_key {
                        Value::String(s) => Some(s.to_str()?.to_string()),
                        Value::Integer(_) => existing_value
                            .as_table()
                            .map(|t| t.get::<Option<String>>("key"))
                            .and_then(Result::ok)
                            .flatten(),
                        _ => None,
                    };
                    if existing.as_deref() == Some(key.as_str()) {
                        return Err(LuaError::runtime(format!("duplicate keybind: {key}")));
                    }
                }
                let entry = lua.create_table()?;
                entry.set("key", key)?;
                // Encode a Lua function reference as a table action so the normal
                // bind-entry parser picks it up via `script_fn_id`.
                let register_fn = |f: mlua::Function| -> LuaResult<LuaTable> {
                    let id = next_bind_fn_id.fetch_add(1, Ordering::SeqCst);
                    bind_functions.lock().unwrap().insert(id, f);
                    let fn_table = lua.create_table()?;
                    fn_table.set("script_fn_id", id)?;
                    Ok(fn_table)
                };
                match action {
                    Value::Function(f) => {
                        entry.set("action", register_fn(f)?)?;
                    }
                    Value::Table(t) => {
                        // Functions can also be given as press and release actions.
                        for slot in ["press", "release"] {
                            if let Value::Function(f) = t.get::<Value>(slot)? {
                                t.set(slot, register_fn(f)?)?;
                            }
                        }
                        entry.set("action", t)?;
                    }
                    other => {
                        entry.set("action", other)?;
                    }
                }
                binds.push(entry)?;
                Ok(())
            })
            .map_err(err)?;
        neori.set("bind", func).map_err(err)?;
    }

    // neori.action(name, args?) dispatches an IPC Action at runtime from inside a Lua
    // callback. The action is appended to `pending_actions`; the compositor drains it
    // after the callback returns.
    //
    // `name` is any action identifier accepted by `parse_action_string` ("close-window",
    // "focus-column-left", "focus-workspace", "spawn", "create-virtual-output", …) or by
    // `action_from_table_key` (the snake/kebab form used in bind tables). `args` is an
    // optional table; it accepts the same shape as `neori.bind("key", { <name> = <args> })`.
    //
    //     neori.action("focus-column-left")
    //     neori.action("spawn", { "kitty", "-e", "sh" })
    //     neori.action("focus-workspace", 2)
    //     neori.action("create-virtual-output", { name = "sunshine", width = 2560 })
    {
        let pending_actions = Arc::clone(pending_actions);
        let func = lua
            .create_function(move |lua, args: mlua::Variadic<Value>| {
                let mut iter = args.into_iter();
                let name = match iter.next() {
                    Some(Value::String(s)) => s
                        .to_str()
                        .map_err(|e| LuaError::runtime(format!("{e}")))?
                        .to_string(),
                    Some(_) => {
                        return Err(LuaError::runtime(
                            "neori.action: first argument must be a string",
                        ))
                    }
                    None => {
                        return Err(LuaError::runtime(
                            "neori.action: expected an action name",
                        ))
                    }
                };
                let arg = iter.next().unwrap_or(Value::Nil);
                let extra = iter.next();
                if extra.is_some() {
                    return Err(LuaError::runtime(
                        "neori.action: too many arguments (expected name and optional args)",
                    ));
                }
                let effective_arg = if matches!(arg, Value::Nil) {
                    Value::Boolean(true)
                } else {
                    arg
                };
                let wrapper = lua.create_table()?;
                wrapper.set(name.as_str(), effective_arg.clone())?;
                let action = resolve_runtime_action(&name, &effective_arg, &wrapper)
                    .map_err(LuaError::runtime)?;
                pending_actions.lock().unwrap().push(action);
                Ok(())
            })
            .map_err(err)?;
        neori.set("action", func).map_err(err)?;
    }

    // neori.windows() returns an array of Window userdata snapshotting every mapped
    // window at the time of the call. The compositor refreshes the backing cache before
    // every Lua callback so each call sees current state; outside a callback the array
    // is empty.
    //
    // neori.focused_window() returns the focused Window, or nil if nothing is focused.
    {
        let cache = Arc::clone(window_cache);
        let func = lua
            .create_function(move |lua, ()| {
                let snapshots = cache.lock().unwrap().windows.clone();
                let arr = lua.create_table()?;
                for (i, snap) in snapshots.into_iter().enumerate() {
                    arr.set(i + 1, Window(snap))?;
                }
                Ok(arr)
            })
            .map_err(err)?;
        neori.set("windows", func).map_err(err)?;
    }
    {
        let cache = Arc::clone(window_cache);
        let func = lua
            .create_function(move |_lua, ()| {
                let focused = cache.lock().unwrap().focused.clone();
                Ok(focused.map(Window))
            })
            .map_err(err)?;
        neori.set("focused_window", func).map_err(err)?;
    }

    // neori.focused_output() — paired with neori.outputs(), which is registered later
    // so it can override the "outputs" section-setter below.
    {
        let cache = Arc::clone(output_cache);
        let func = lua
            .create_function(move |_lua, ()| {
                let focused = cache.lock().unwrap().focused.clone();
                Ok(focused.map(LuaOutput))
            })
            .map_err(err)?;
        neori.set("focused_output", func).map_err(err)?;
    }

    // neori.focused_workspace() — paired with neori.workspaces() below.
    {
        let cache = Arc::clone(workspace_cache);
        let func = lua
            .create_function(move |_lua, ()| {
                let focused = cache.lock().unwrap().focused.clone();
                Ok(focused.map(LuaWorkspace))
            })
            .map_err(err)?;
        neori.set("focused_workspace", func).map_err(err)?;
    }

    // neori.pointer() returns { x, y, output } or nil if the cache is unseeded.
    {
        let cache = Arc::clone(pointer_cache);
        let func = lua
            .create_function(move |lua, ()| {
                let snapshot = cache.lock().unwrap().clone();
                match snapshot {
                    Some(s) => {
                        let t = lua.create_table()?;
                        t.set("x", s.x)?;
                        t.set("y", s.y)?;
                        if let Some(name) = s.output_name {
                            t.set("output", name)?;
                        }
                        Ok(mlua::Value::Table(t))
                    }
                    None => Ok(mlua::Value::Nil),
                }
            })
            .map_err(err)?;
        neori.set("pointer", func).map_err(err)?;
    }

    // neori.warp(x, y) enqueues a cursor warp to the given global coordinates.
    // The compositor drains the queue after the callback returns and dispatches
    // each warp via `State::move_cursor`.
    {
        let queue = Arc::clone(pending_warps);
        let func = lua
            .create_function(move |_lua, (x, y): (f64, f64)| {
                if !x.is_finite() || !y.is_finite() {
                    return Err(LuaError::runtime(format!(
                        "neori.warp: non-finite coordinates ({x}, {y})"
                    )));
                }
                queue.lock().unwrap().push((x, y));
                Ok(())
            })
            .map_err(err)?;
        neori.set("warp", func).map_err(err)?;
    }

    // neori.keyboard_layout() returns the active layout's name (as it appears in
    // the keymap), or nil if the cache is unseeded.
    {
        let cache = Arc::clone(keyboard_layout);
        let func = lua
            .create_function(move |_lua, ()| Ok(cache.lock().unwrap().clone()))
            .map_err(err)?;
        neori.set("keyboard_layout", func).map_err(err)?;
    }

    // neori.devices() returns an array of Device userdata — one per connected
    // libinput device. Empty when the cache is unseeded or there are no devices.
    {
        let cache = Arc::clone(device_cache);
        let func = lua
            .create_function(move |lua, ()| {
                let snapshots = cache.lock().unwrap().devices.clone();
                let arr = lua.create_table()?;
                for (i, snap) in snapshots.into_iter().enumerate() {
                    arr.set(i + 1, LuaDevice(snap))?;
                }
                Ok(arr)
            })
            .map_err(err)?;
        neori.set("devices", func).map_err(err)?;
    }

    // neori.spawn(...) / neori.spawn_sh(...) build spawn actions usable in neori.bind and
    // neori.on. `neori.spawn("kitty")` and `neori.spawn { "kitty", "-e", "sh" }` build an
    // argv command; `neori.spawn_sh("cmd | cmd")` builds a shell command. An optional
    // trailing table with string keys is merged in as bind metadata, e.g.
    // `neori.spawn("kitty", { hotkey_overlay_title = "Terminal", ["repeat"] = false })`.
    for (name, shell) in [("spawn", false), ("spawn_sh", true)] {
        let func = lua
            .create_function(move |lua, args: mlua::Variadic<Value>| {
                spawn_action(lua, args.into_iter().collect(), shell).map_err(LuaError::runtime)
            })
            .map_err(err)?;
        neori.set(name, func).map_err(err)?;
    }

    // neori.create_virtual_output(opts) and neori.remove_virtual_output(name | opts) build
    // action tables usable in neori.bind and neori.on. All options are optional for the
    // "create" form:
    //
    //     neori.create_virtual_output { name = "sunshine", width = 1920, height = 1080,
    //                                   refresh_rate = 144 }
    //     neori.remove_virtual_output("sunshine")
    //     neori.remove_virtual_output { name = "sunshine" }
    //
    // Any extra string-keyed fields (hotkey_overlay_title, cooldown_ms, repeat, …) are merged
    // in as bind metadata, matching `neori.spawn`'s behavior.
    {
        let func = lua
            .create_function(|lua, args: mlua::Variadic<Value>| {
                create_virtual_output_action(lua, args.into_iter().collect())
                    .map_err(LuaError::runtime)
            })
            .map_err(err)?;
        neori.set("create_virtual_output", func).map_err(err)?;
    }
    {
        let func = lua
            .create_function(|lua, args: mlua::Variadic<Value>| {
                remove_virtual_output_action(lua, args.into_iter().collect())
                    .map_err(LuaError::runtime)
            })
            .map_err(err)?;
        neori.set("remove_virtual_output", func).map_err(err)?;
    }

    // Whole-section setters: neori.input { ... }, neori.layout { ... } and so on.
    //
    // "outputs" and "workspaces" are deliberately absent here: the collision-free
    // spellings for declaring entries are `neori.output("name", { … })` /
    // `neori.workspace("name", { … })`, and the plural names are runtime lookup
    // functions instead. Shipped configs always use the singular entry form.
    for name in [
        "binds",
        "input",
        "layout",
        "cursor",
        "hotkey_overlay",
        "config_notification",
        "clipboard",
        "animations",
        "blur",
        "gestures",
        "overview",
        "xwayland_satellite",
        "switch_events",
        "debug",
        "recent_windows",
        "environment",
        "prefer_no_csd",
        "screenshot_path",
        "window_rules",
        "layer_rules",
        "scratchpads",
    ] {
        let data = data.clone();
        let func = lua
            .create_function(move |_, value: Value| {
                data.set(name, value)?;
                Ok(())
            })
            .map_err(err)?;
        neori.set(name, func).map_err(err)?;
    }

    // neori.outputs() — runtime lookup, returns an array of Output userdata snapshots.
    // Registered after the section-setter loop so it is the final binding under
    // `neori.outputs`.
    {
        let cache = Arc::clone(output_cache);
        let func = lua
            .create_function(move |lua, ()| {
                let snapshots = cache.lock().unwrap().outputs.clone();
                let arr = lua.create_table()?;
                for (i, snap) in snapshots.into_iter().enumerate() {
                    arr.set(i + 1, LuaOutput(snap))?;
                }
                Ok(arr)
            })
            .map_err(err)?;
        neori.set("outputs", func).map_err(err)?;
    }

    // neori.workspaces() — same pattern as outputs(). Registered after the section
    // setter loop (which also owned the "workspaces" name) for the same reason.
    {
        let cache = Arc::clone(workspace_cache);
        let func = lua
            .create_function(move |lua, ()| {
                let snapshots = cache.lock().unwrap().workspaces.clone();
                let arr = lua.create_table()?;
                for (i, snap) in snapshots.into_iter().enumerate() {
                    arr.set(i + 1, LuaWorkspace(snap))?;
                }
                Ok(arr)
            })
            .map_err(err)?;
        neori.set("workspaces", func).map_err(err)?;
    }

    // Entry pushers: neori.window_rule { ... } appends one entry to a section.
    //
    // `neori.output` and `neori.workspace` also accept a leading name argument, i.e.
    // `neori.output("eDP-1", { scale = 2.0 })`, which is sugar for `neori.output { name = "eDP-1",
    // scale = 2.0 }`.
    for (name, section, named) in [
        ("window_rule", "window_rules", false),
        ("layer_rule", "layer_rules", false),
        ("output", "outputs", true),
        ("workspace", "workspaces", true),
        ("scratchpad", "scratchpads", true),
    ] {
        let data = data.clone();
        let func = lua
            .create_function(move |lua, args: mlua::Variadic<Value>| {
                let mut args = args.into_iter();
                let first = args.next().ok_or_else(|| {
                    LuaError::runtime(format!("neori.{name}: expected an argument"))
                })?;
                let value = match (args.next(), args.next()) {
                    (None, None) => first,
                    (Some(rest), None) if named => {
                        let name_val = match first {
                            Value::String(s) => s,
                            _ => {
                                return Err(LuaError::runtime(format!(
                                    "neori.{name}(name, table): name must be a string"
                                )))
                            }
                        };
                        let table = match rest {
                            Value::Table(t) => t,
                            _ => {
                                return Err(LuaError::runtime(format!(
                                    "neori.{name}(name, table): second argument must be a table"
                                )))
                            }
                        };
                        table.set("name", name_val)?;
                        Value::Table(table)
                    }
                    _ => {
                        return Err(LuaError::runtime(format!(
                            "neori.{name}: expected a single table argument"
                        )))
                    }
                };
                push_section_entry(lua, &data, section, value)?;
                Ok(())
            })
            .map_err(err)?;
        neori.set(name, func).map_err(err)?;
    }

    // neori.on(event, handler) registers an event handler.
    //
    // String and table handlers are recorded into the `data` table and parsed into
    // event-specific `Config` fields (`startup`, `monitor_added`, `monitor_removed`); function
    // handlers are kept in the persistent Lua host and invoked by the compositor when the event
    // fires.
    {
        let data = data.clone();
        let event_functions = Arc::clone(event_functions);
        let func = lua
            .create_function(move |lua, (event, handler): (String, Value)| {
                let event = match event.as_str() {
                    "startup" => "startup",
                    "monitor-added" | "monitor_added" => "monitor-added",
                    "monitor-removed" | "monitor_removed" => "monitor-removed",
                    "lid-open" | "lid_open" => "lid-open",
                    "lid-closed" | "lid_closed" => "lid-closed",
                    "tablet-mode-on" | "tablet_mode_on" => "tablet-mode-on",
                    "tablet-mode-off" | "tablet_mode_off" => "tablet-mode-off",
                    "window-opening" | "window_opening" => "window-opening",
                    "window-opened" | "window_opened" => "window-opened",
                    "window-closed" | "window_closed" => "window-closed",
                    "window-focused" | "window_focused" => "window-focused",
                    "workspace-focused" | "workspace_focused" => "workspace-focused",
                    "workspace-added" | "workspace_added" => "workspace-added",
                    "workspace-removed" | "workspace_removed" => "workspace-removed",
                    "output-focused" | "output_focused" => "output-focused",
                    "layout-changed" | "layout_changed" => "layout-changed",
                    _ => {
                        return Err(LuaError::runtime(format!(
                            "unknown event '{event}' (supported events: startup, monitor-added, \
                             monitor-removed, lid-open, lid-closed, tablet-mode-on, \
                             tablet-mode-off, window-opening, window-opened, window-closed, \
                             window-focused, workspace-focused, workspace-added, \
                             workspace-removed, output-focused, layout-changed)"
                        )));
                    }
                };
                match handler {
                    Value::Function(f) => {
                        event_functions
                            .lock()
                            .unwrap()
                            .entry(event.to_string())
                            .or_default()
                            .push(f);
                        Ok(())
                    }
                    Value::String(_) | Value::Table(_) => {
                        let entries: Option<LuaTable> = data.get(event)?;
                        let entries = match entries {
                            Some(entries) => entries,
                            None => {
                                let entries = lua.create_table()?;
                                data.set(event, entries.clone())?;
                                entries
                            }
                        };
                        entries.push(handler)?;
                        Ok(())
                    }
                    _ => Err(LuaError::runtime(
                        "event handler must be a string, a table, or a function",
                    )),
                }
            })
            .map_err(err)?;
        neori.set("on", func).map_err(err)?;
    }

    lua.globals().set("neori", neori).map_err(err)?;
    Ok(data)
}

/// Appends `value` to the sequence stored at `data[section]`, creating it if needed.
fn push_section_entry(
    lua: &Lua,
    data: &LuaTable,
    section: &str,
    value: impl IntoLua,
) -> LuaResult<()> {
    let entries: Option<LuaTable> = data.get(section)?;
    let entries = match entries {
        Some(entries) => entries,
        None => {
            let entries = lua.create_table()?;
            data.set(section, entries.clone())?;
            entries
        }
    };
    entries.push(value)?;
    Ok(())
}

/// Merges the data collected through the `neori` API into a returned config table.
///
/// Keys already present in the returned table take precedence, except for `binds`, which is
/// merged bind-by-bind so that both styles can contribute binds.
fn merge_api_data(table: &LuaTable, data: &LuaTable) -> Result<(), String> {
    for pair in data.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("{e}"))?;
        let existing: Value = table.get(key.clone()).map_err(|e| format!("{e}"))?;
        if matches!(existing, Value::Nil) {
            table.set(key, value).map_err(|e| format!("{e}"))?;
            continue;
        }

        if let (Value::String(name), Value::Table(api), Value::Table(returned)) =
            (&key, &value, &existing)
        {
            if name.to_str().map_err(|e| format!("{e}"))? == "binds" {
                for pair in api.pairs::<Value, Value>() {
                    let (bkey, bvalue) = pair.map_err(|e| format!("{e}"))?;
                    let bind_key = match &bkey {
                        Value::String(s) => {
                            Some(s.to_str().map_err(|e| format!("{e}"))?.to_string())
                        }
                        Value::Integer(_) => bvalue
                            .as_table()
                            .map(|t| t.get::<Option<String>>("key"))
                            .and_then(Result::ok)
                            .flatten(),
                        _ => None,
                    };
                    if let Some(bind_key) = bind_key {
                        if table_has_bind(returned, &bind_key)? {
                            continue;
                        }
                    }
                    if matches!(bkey, Value::Integer(_)) {
                        returned.push(bvalue.clone()).map_err(|e| format!("{e}"))?;
                    } else {
                        returned.set(bkey, bvalue).map_err(|e| format!("{e}"))?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Checks whether the given binds table already contains a bind for `key`, in either
/// map form (`["Mod+P"] = ...`) or sequence form (`{ key = "Mod+P", action = ... }`).
fn table_has_bind(binds: &LuaTable, key: &str) -> Result<bool, String> {
    let direct: Value = binds.get(key).map_err(|e| format!("{e}"))?;
    if !matches!(direct, Value::Nil) {
        return Ok(true);
    }
    for pair in binds.clone().pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| format!("{e}"))?;
        if matches!(k, Value::Integer(_)) {
            if let Value::Table(t) = &v {
                let entry_key: Option<String> =
                    t.get("key").map_err(|e| format!("{e}")).ok().flatten();
                if entry_key.as_deref() == Some(key) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Top-level config keys recognized by [`parse_table_into_config`].
///
/// Used to emit a `warn!` for unknown keys (typos) so they don't get silently dropped.
/// Both snake_case and kebab-case aliases must be listed here.
const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
    "input",
    "layout",
    "cursor",
    "hotkey_overlay",
    "hotkey-overlay",
    "config_notification",
    "config-notification",
    "clipboard",
    "animations",
    "blur",
    "gestures",
    "overview",
    "xwayland_satellite",
    "xwayland-satellite",
    "binds",
    "switch_events",
    "switch-events",
    "debug",
    "recent_windows",
    "recent-windows",
    "environment",
    "startup",
    "monitor_added",
    "monitor-added",
    "monitor_removed",
    "monitor-removed",
    "lid_open",
    "lid-open",
    "lid_closed",
    "lid-closed",
    "tablet_mode_on",
    "tablet-mode-on",
    "tablet_mode_off",
    "tablet-mode-off",
    "outputs",
    "output",
    "window_rules",
    "window-rules",
    "layer_rules",
    "layer-rules",
    "workspaces",
    "workspace",
    "scratchpads",
    "scratchpad",
    "prefer_no_csd",
    "prefer-no-csd",
    "screenshot_path",
    "screenshot-path",
];

thread_local! {
    /// Whether unknown keys should be rejected instead of warned about.
    ///
    /// Set from `NEORI_STRICT_CONFIG` at the start of each [`ScriptHost::run`] and toggled by
    /// the `neori.strict()` API. Thread-local so parallel test runs don't interfere.
    static STRICT_CONFIG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Files that the config depends on, watched for changes to reload the config.
type WatchedFiles = Arc<Mutex<Vec<PathBuf>>>;

thread_local! {
    /// The config file's directory and the shared list of watched files, available while
    /// [`ScriptHost::run`] converts the evaluated Lua table into a [`Config`].
    ///
    /// Options that reference other files (like `custom_shader = { path = ... }`) resolve
    /// relative paths against the directory and register the file for the config watcher.
    static PARSE_ENV: std::cell::RefCell<Option<(PathBuf, WatchedFiles)>> =
        const { std::cell::RefCell::new(None) };
}

/// Resolves a path from the config: `~` expands to the home directory, and relative paths
/// are taken relative to the config file's directory.
fn resolve_config_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if let Ok(rest) = path.strip_prefix("~") {
        let home = std::env::home_dir()
            .ok_or_else(|| format!("error retrieving home directory to expand {path:?}"))?;
        return Ok(home.join(rest));
    }
    let base = PARSE_ENV.with(|env| env.borrow().as_ref().map(|(dir, _)| dir.clone()));
    Ok(match base {
        Some(base) => base.join(path),
        None => path.to_path_buf(),
    })
}

/// Registers a file that the config depends on, so that changing it reloads the config.
fn watch_config_file(path: &Path) {
    PARSE_ENV.with(|env| {
        if let Some((_, includes)) = env.borrow().as_ref() {
            let mut includes = includes.lock().unwrap();
            if !includes.iter().any(|p| p == path) {
                includes.push(path.to_path_buf());
            }
        }
    });
}

/// Returns whether strict config parsing is enabled in the current thread.
fn strict_config() -> bool {
    STRICT_CONFIG.with(|c| c.get())
}

/// Enables or disables strict config parsing in the current thread.
fn set_strict_config(value: bool) {
    STRICT_CONFIG.with(|c| c.set(value));
}

/// Reports an unknown key in a config section.
///
/// Emits a warning by default, or an error when strict config parsing is enabled.
fn unknown_key(section: &str, key: &str) -> Result<(), String> {
    if strict_config() {
        Err(format!("unknown key `{key}` in {section}"))
    } else {
        warn!("neori-config: unknown key `{key}` in {section}");
        Ok(())
    }
}

/// Reports a known config key used with a value type it does not accept.
///
/// Emits a warning by default, or an error when strict config parsing is enabled.
fn invalid_config(message: &str) -> Result<(), String> {
    if strict_config() {
        Err(message.to_string())
    } else {
        warn!("neori-config: {message}");
        Ok(())
    }
}

fn parse_table_into_config(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error reading config key: {e}"))?;
        let key_str = key_str(&key)?;

        if !KNOWN_TOP_LEVEL_KEYS.contains(&key_str.as_str()) {
            unknown_key("top-level config", &key_str)?;
            continue;
        }

        match value {
            Value::Table(t) => match key_str.as_str() {
                "input" => parse_input_section(t, config)?,
                "layout" => parse_layout_section(t, config)?,
                "cursor" => parse_cursor_section(t, config)?,
                "hotkey_overlay" | "hotkey-overlay" => parse_hotkey_overlay_section(t, config)?,
                "config_notification" | "config-notification" => {
                    parse_config_notification_section(t, config)?;
                }
                "clipboard" => parse_clipboard_section(t, config)?,
                "animations" => parse_animations_section(t, config)?,
                "blur" => parse_blur_section(t, config)?,
                "gestures" => parse_gestures_section(t, config)?,
                "overview" => parse_overview_section(t, config)?,
                "xwayland_satellite" | "xwayland-satellite" => {
                    parse_xwayland_satellite_section(t, config)?;
                }
                "binds" => parse_binds_section(t, config)?,
                "switch_events" | "switch-events" => parse_switch_events_section(t, config)?,
                "debug" => parse_debug_section(t, config)?,
                "recent_windows" | "recent-windows" => parse_recent_windows_section(t, config)?,
                "environment" => parse_environment_section(t, config)?,
                "startup" => config.startup = parse_event_actions(&t)?,
                "monitor_added" | "monitor-added" => {
                    config.monitor_added = parse_event_actions(&t)?
                }
                "monitor_removed" | "monitor-removed" => {
                    config.monitor_removed = parse_event_actions(&t)?
                }
                "lid_open" | "lid-open" => config.lid_open = parse_event_actions(&t)?,
                "lid_closed" | "lid-closed" => config.lid_closed = parse_event_actions(&t)?,
                "tablet_mode_on" | "tablet-mode-on" => {
                    config.tablet_mode_on = parse_event_actions(&t)?
                }
                "tablet_mode_off" | "tablet-mode-off" => {
                    config.tablet_mode_off = parse_event_actions(&t)?
                }
                "outputs" | "output" => parse_outputs(t, config)?,
                "window_rules" | "window-rules" => parse_window_rules(t, config)?,
                "layer_rules" | "layer-rules" => parse_layer_rules(t, config)?,
                "workspaces" | "workspace" => parse_workspaces(t, config)?,
                "scratchpads" | "scratchpad" => parse_scratchpads(t, config)?,
                _ => invalid_config(&format!("key `{key_str}` does not accept a table value"))?,
            },
            Value::Boolean(b) => match key_str.as_str() {
                "prefer_no_csd" | "prefer-no-csd" => config.prefer_no_csd = b,
                _ => invalid_config(&format!("key `{key_str}` does not accept a boolean value"))?,
            },
            Value::Nil => match key_str.as_str() {
                "screenshot_path" | "screenshot-path" => {
                    config.screenshot_path = ScreenshotPath(None);
                }
                _ => invalid_config(&format!("key `{key_str}` does not accept a nil value"))?,
            },
            _ => match key_str.as_str() {
                "prefer_no_csd" | "prefer-no-csd" => config.prefer_no_csd = value_is_truthy(&value),
                "screenshot_path" | "screenshot-path" => {
                    let s = value_to_string(&value)?;
                    config.screenshot_path = ScreenshotPath(Some(s));
                }
                _ => invalid_config(&format!(
                    "key `{key_str}` has unsupported value type {:?}",
                    value.type_name()
                ))?,
            },
        }
    }
    Ok(())
}

/// Parses a sequence of event handler actions (strings or action tables) from a Lua table.
fn parse_event_actions(t: &LuaTable) -> Result<Vec<Action>, String> {
    let mut actions = Vec::new();
    for pair in t.clone().pairs::<Value, Value>() {
        let (_, handler) = pair.map_err(|e| format!("event error: {e}"))?;
        actions.push(parse_startup_action(&handler)?);
    }
    Ok(actions)
}

fn key_str(val: &Value) -> Result<String, String> {
    match val {
        Value::String(s) => Ok(s.to_str().map_err(|e| format!("{e}"))?.to_string()),
        _ => Err(format!("expected string key, got {:?}", val.type_name())),
    }
}

fn value_is_truthy(val: &Value) -> bool {
    match val {
        Value::Nil => false,
        Value::Boolean(b) => *b,
        _ => true,
    }
}

fn value_to_string(val: &Value) -> Result<String, String> {
    match val {
        Value::String(s) => Ok(s.to_str().map_err(|e| format!("{e}"))?.to_string()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(f) => Ok(f.to_string()),
        _ => Err(format!("expected string value, got {:?}", val.type_name())),
    }
}

fn value_to_f64(val: &Value) -> Result<f64, String> {
    match val {
        Value::Integer(i) => Ok(*i as f64),
        Value::Number(f) => Ok(*f),
        Value::String(s) => {
            let s = s.to_str().map_err(|e| format!("{e}"))?;
            s.parse::<f64>().map_err(|_| format!("invalid number: {s}"))
        }
        _ => Err(format!("expected number, got {:?}", val.type_name())),
    }
}

fn value_to_i64(val: &Value) -> Result<i64, String> {
    match val {
        Value::Integer(i) => Ok(*i),
        Value::Number(f) => Ok(*f as i64),
        Value::String(s) => {
            let s = s.to_str().map_err(|e| format!("{e}"))?;
            s.parse::<i64>()
                .map_err(|_| format!("invalid integer: {s}"))
        }
        _ => Err(format!("expected integer, got {:?}", val.type_name())),
    }
}

fn value_to_u8(val: &Value) -> Result<u8, String> {
    let i = value_to_i64(val)?;
    u8::try_from(i).map_err(|_| format!("value out of range for u8: {i}"))
}

fn value_to_u16(val: &Value) -> Result<u16, String> {
    let i = value_to_i64(val)?;
    u16::try_from(i).map_err(|_| format!("value out of range for u16: {i}"))
}

fn value_to_default_preset_size(val: &Value) -> Result<DefaultPresetSize, String> {
    match val {
        Value::Table(t) => {
            if let Ok(v) = t.get::<f64>("proportion") {
                Ok(DefaultPresetSize(Some(PresetSize::Proportion(v))))
            } else if let Ok(v) = t.get::<f64>("fixed") {
                Ok(DefaultPresetSize(Some(PresetSize::Fixed(v as i32))))
            } else {
                // An empty table means: let windows decide their own size.
                Ok(DefaultPresetSize(None))
            }
        }
        _ => Err(format!(
            "expected a table with proportion or fixed, got {:?}",
            val.type_name()
        )),
    }
}

fn value_to_u32(val: &Value) -> Result<u32, String> {
    let i = value_to_i64(val)?;
    u32::try_from(i).map_err(|_| format!("value out of range for u32: {i}"))
}

fn value_to_u64(val: &Value) -> Result<u64, String> {
    let i = value_to_i64(val)?;
    u64::try_from(i).map_err(|_| format!("value out of range for u64: {i}"))
}

fn parse_css_color(s: &str) -> Result<Color, String> {
    csscolorparser::parse(s)
        .map(|c| Color {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        })
        .map_err(|e| format!("invalid color '{s}': {e}"))
}

fn parse_input_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in input section: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "keyboard" => {
                if let Value::Table(t) = &value {
                    parse_keyboard(t, &mut config.input.keyboard)?;
                }
            }
            "touchpad" => {
                if let Value::Table(t) = &value {
                    parse_touchpad(t, &mut config.input.touchpad)?;
                }
            }
            "mouse" => {
                if let Value::Table(t) = &value {
                    parse_mouse(t, &mut config.input.mouse)?;
                }
            }
            "trackpoint" => {
                if let Value::Table(t) = &value {
                    parse_trackpoint(t, &mut config.input.trackpoint)?;
                }
            }
            "trackball" => {
                if let Value::Table(t) = &value {
                    parse_trackball(t, &mut config.input.trackball)?;
                }
            }
            "tablet" => {
                if let Value::Table(t) = &value {
                    parse_tablet(t, &mut config.input.tablet)?;
                }
            }
            "touch" => {
                if let Value::Table(t) = &value {
                    parse_touch(t, &mut config.input.touch)?;
                }
            }
            "disable_power_key_handling" | "disable-power-key-handling" => {
                config.input.disable_power_key_handling = value_is_truthy(&value);
            }
            "workspace_auto_back_and_forth" | "workspace-auto-back-and-forth" => {
                config.input.workspace_auto_back_and_forth = value_is_truthy(&value);
            }
            "mod_key" | "mod-key" => {
                let s = value_to_string(&value)?;
                config.input.mod_key = Some(
                    s.parse::<ModKey>()
                        .map_err(|e| format!("invalid mod-key: {e}"))?,
                );
            }
            "mod_key_nested" | "mod-key-nested" => {
                let s = value_to_string(&value)?;
                config.input.mod_key_nested = Some(
                    s.parse::<ModKey>()
                        .map_err(|e| format!("invalid mod-key-nested: {e}"))?,
                );
            }
            "focus_follows_mouse" | "focus-follows-mouse" => {
                if let Value::Table(t) = &value {
                    let max_scroll: Option<String> =
                        t.get("max_scroll_amount").map_err(|e| format!("{e}")).ok();
                    let max_scroll_amount = max_scroll
                        .as_deref()
                        .and_then(|s| s.parse::<Percent>().ok());
                    config.input.focus_follows_mouse =
                        Some(FocusFollowsMouse { max_scroll_amount });
                } else if value_is_truthy(&value) {
                    config.input.focus_follows_mouse = Some(FocusFollowsMouse {
                        max_scroll_amount: None,
                    });
                }
            }
            "warp_mouse_to_focus" | "warp-mouse-to-focus" => {
                if value_is_truthy(&value) {
                    let mode = if let Value::Table(t) = &value {
                        let s: Option<String> = t.get("mode").map_err(|e| format!("{e}")).ok();
                        s.and_then(|s| s.parse::<WarpMouseToFocusMode>().ok())
                    } else {
                        None
                    };
                    config.input.warp_mouse_to_focus = Some(WarpMouseToFocus { mode });
                }
            }
            _ => unknown_key("input", &ks)?,
        }
    }
    Ok(())
}

fn parse_keyboard(table: &LuaTable, kb: &mut Keyboard) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in keyboard section: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "xkb" => {
                if let Value::Table(t) = &value {
                    kb.xkb.rules = t.get("rules").unwrap_or_default();
                    kb.xkb.model = t.get("model").unwrap_or_default();
                    kb.xkb.layout = t.get("layout").unwrap_or_default();
                    kb.xkb.variant = t.get("variant").unwrap_or_default();
                    kb.xkb.options = t.get("options").unwrap_or(None);
                    let file: Option<String> = t.get("file").unwrap_or(None);
                    kb.xkb.file = file.map(|file| read_xkb_file(&file));
                }
            }
            "repeat_delay" | "repeat-delay" => {
                if let Ok(v) = value_to_u16(&value) {
                    kb.repeat_delay = v;
                }
            }
            "repeat_rate" | "repeat-rate" => {
                if let Ok(v) = value_to_u8(&value) {
                    kb.repeat_rate = v;
                }
            }
            "track_layout" | "track-layout" => {
                let s = value_to_string(&value)?;
                kb.track_layout = match s.as_str() {
                    "global" => TrackLayout::Global,
                    "window" => TrackLayout::Window,
                    _ => {
                        return Err(format!(
                            "invalid track-layout: {s}, expected 'global' or 'window'"
                        ))
                    }
                };
            }
            "numlock" => kb.numlock = value_is_truthy(&value),
            _ => unknown_key("input.keyboard", &ks)?,
        }
    }
    Ok(())
}

fn parse_touchpad(table: &LuaTable, tp: &mut Touchpad) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in touchpad: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => tp.off = value_is_truthy(&value),
            "tap" => tp.tap = value_is_truthy(&value),
            "dwt" => tp.dwt = value_is_truthy(&value),
            "dwtp" => tp.dwtp = value_is_truthy(&value),
            "drag" => tp.drag = Some(value_is_truthy(&value)),
            "drag_lock" | "drag-lock" => tp.drag_lock = value_is_truthy(&value),
            "natural_scroll" | "natural-scroll" => tp.natural_scroll = value_is_truthy(&value),
            "click_method" | "click-method" => {
                let s = value_to_string(&value)?;
                tp.click_method = Some(
                    s.parse::<ClickMethod>()
                        .map_err(|e| format!("invalid click-method: {e}"))?,
                );
            }
            "accel_speed" | "accel-speed" => tp.accel_speed = FloatOrInt(value_to_f64(&value)?),
            "accel_profile" | "accel-profile" => {
                let s = value_to_string(&value)?;
                tp.accel_profile = Some(
                    s.parse::<AccelProfile>()
                        .map_err(|e| format!("invalid accel-profile: {e}"))?,
                );
            }
            "scroll_method" | "scroll-method" => {
                let s = value_to_string(&value)?;
                tp.scroll_method = Some(
                    s.parse::<ScrollMethod>()
                        .map_err(|e| format!("invalid scroll-method: {e}"))?,
                );
            }
            "scroll_button" | "scroll-button" => tp.scroll_button = Some(value_to_u32(&value)?),
            "scroll_button_lock" | "scroll-button-lock" => {
                tp.scroll_button_lock = value_is_truthy(&value);
            }
            "tap_button_map" | "tap-button-map" => {
                let s = value_to_string(&value)?;
                tp.tap_button_map = Some(
                    s.parse::<TapButtonMap>()
                        .map_err(|e| format!("invalid tap-button-map: {e}"))?,
                );
            }
            "left_handed" | "left-handed" => tp.left_handed = value_is_truthy(&value),
            "disabled_on_external_mouse" | "disabled-on-external-mouse" => {
                tp.disabled_on_external_mouse = value_is_truthy(&value);
            }
            "middle_emulation" | "middle-emulation" => {
                tp.middle_emulation = value_is_truthy(&value);
            }
            "scroll_factor" | "scroll-factor" => {
                tp.scroll_factor = Some(parse_scroll_factor(&value)?);
            }
            "pinch_sensitivity" | "pinch-sensitivity" => {
                tp.pinch_sensitivity = parse_pinch_sensitivity(&value, "input.touchpad")?;
            }
            _ => unknown_key("input.touchpad", &ks)?,
        }
    }
    Ok(())
}

fn parse_mouse(table: &LuaTable, mouse: &mut Mouse) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in mouse: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => mouse.off = value_is_truthy(&value),
            "natural_scroll" | "natural-scroll" => mouse.natural_scroll = value_is_truthy(&value),
            "accel_speed" | "accel-speed" => mouse.accel_speed = FloatOrInt(value_to_f64(&value)?),
            "accel_profile" | "accel-profile" => {
                let s = value_to_string(&value)?;
                mouse.accel_profile = Some(
                    s.parse::<AccelProfile>()
                        .map_err(|e| format!("invalid accel-profile: {e}"))?,
                );
            }
            "scroll_method" | "scroll-method" => {
                let s = value_to_string(&value)?;
                mouse.scroll_method = Some(
                    s.parse::<ScrollMethod>()
                        .map_err(|e| format!("invalid scroll-method: {e}"))?,
                );
            }
            "scroll_button" | "scroll-button" => mouse.scroll_button = Some(value_to_u32(&value)?),
            "scroll_button_lock" | "scroll-button-lock" => {
                mouse.scroll_button_lock = value_is_truthy(&value);
            }
            "left_handed" | "left-handed" => mouse.left_handed = value_is_truthy(&value),
            "middle_emulation" | "middle-emulation" => {
                mouse.middle_emulation = value_is_truthy(&value);
            }
            "scroll_factor" | "scroll-factor" => {
                mouse.scroll_factor = Some(parse_scroll_factor(&value)?);
            }
            _ => unknown_key("input.mouse", &ks)?,
        }
    }
    Ok(())
}

fn parse_trackpoint(table: &LuaTable, tp: &mut Trackpoint) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in trackpoint: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => tp.off = value_is_truthy(&value),
            "natural_scroll" | "natural-scroll" => tp.natural_scroll = value_is_truthy(&value),
            "accel_speed" | "accel-speed" => tp.accel_speed = FloatOrInt(value_to_f64(&value)?),
            "accel_profile" | "accel-profile" => {
                let s = value_to_string(&value)?;
                tp.accel_profile = Some(
                    s.parse::<AccelProfile>()
                        .map_err(|e| format!("invalid accel-profile: {e}"))?,
                );
            }
            "scroll_method" | "scroll-method" => {
                let s = value_to_string(&value)?;
                tp.scroll_method = Some(
                    s.parse::<ScrollMethod>()
                        .map_err(|e| format!("invalid scroll-method: {e}"))?,
                );
            }
            "scroll_button" | "scroll-button" => tp.scroll_button = Some(value_to_u32(&value)?),
            "scroll_button_lock" | "scroll-button-lock" => {
                tp.scroll_button_lock = value_is_truthy(&value);
            }
            "middle_emulation" | "middle-emulation" => {
                tp.middle_emulation = value_is_truthy(&value);
            }
            _ => unknown_key("input.trackpoint", &ks)?,
        }
    }
    Ok(())
}

fn parse_trackball(table: &LuaTable, tb: &mut Trackball) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in trackball: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => tb.off = value_is_truthy(&value),
            "natural_scroll" | "natural-scroll" => tb.natural_scroll = value_is_truthy(&value),
            "accel_speed" | "accel-speed" => tb.accel_speed = FloatOrInt(value_to_f64(&value)?),
            "accel_profile" | "accel-profile" => {
                let s = value_to_string(&value)?;
                tb.accel_profile = Some(
                    s.parse::<AccelProfile>()
                        .map_err(|e| format!("invalid accel-profile: {e}"))?,
                );
            }
            "scroll_method" | "scroll-method" => {
                let s = value_to_string(&value)?;
                tb.scroll_method = Some(
                    s.parse::<ScrollMethod>()
                        .map_err(|e| format!("invalid scroll-method: {e}"))?,
                );
            }
            "scroll_button" | "scroll-button" => tb.scroll_button = Some(value_to_u32(&value)?),
            "scroll_button_lock" | "scroll-button-lock" => {
                tb.scroll_button_lock = value_is_truthy(&value);
            }
            "left_handed" | "left-handed" => tb.left_handed = value_is_truthy(&value),
            "middle_emulation" | "middle-emulation" => {
                tb.middle_emulation = value_is_truthy(&value);
            }
            _ => unknown_key("input.trackball", &ks)?,
        }
    }
    Ok(())
}

fn parse_tablet(table: &LuaTable, tablet: &mut Tablet) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in tablet: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => tablet.off = value_is_truthy(&value),
            "calibration_matrix" | "calibration-matrix" => {
                if let Value::Table(t) = &value {
                    let mut vals = Vec::new();
                    for v in t.clone().sequence_values::<f64>() {
                        vals.push(v.map_err(|e| format!("{e}"))? as f32);
                    }
                    tablet.calibration_matrix = Some(vals);
                }
            }
            "map_to_output" | "map-to-output" => {
                tablet.map_to_output = Some(value_to_string(&value)?);
            }
            "map_to_focused_output" | "map-to-focused-output" => {
                tablet.map_to_focused_output = value_is_truthy(&value);
            }
            "map_to_focused_window" | "map-to-focused-window" => {
                tablet.map_to_focused_window = value_is_truthy(&value);
            }
            "left_handed" | "left-handed" => tablet.left_handed = value_is_truthy(&value),
            _ => unknown_key("input.tablet", &ks)?,
        }
    }
    Ok(())
}

fn parse_touch(table: &LuaTable, touch: &mut Touch) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("error in touch: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => touch.off = value_is_truthy(&value),
            "calibration_matrix" | "calibration-matrix" => {
                if let Value::Table(t) = &value {
                    let mut vals = Vec::new();
                    for v in t.clone().sequence_values::<f64>() {
                        vals.push(v.map_err(|e| format!("{e}"))? as f32);
                    }
                    touch.calibration_matrix = Some(vals);
                }
            }
            "map_to_output" | "map-to-output" => {
                touch.map_to_output = Some(value_to_string(&value)?);
            }
            _ => unknown_key("input.touch", &ks)?,
        }
    }
    Ok(())
}

/// Parses a pinch sensitivity multiplier, which must lie within `0..=100`.
fn parse_pinch_sensitivity(
    val: &Value,
    section: &str,
) -> Result<Option<FloatOrInt<0, 100>>, String> {
    let v = value_to_f64(val)?;
    if !(0. ..=100.).contains(&v) {
        invalid_config(&format!(
            "{section}: pinch_sensitivity must be between 0 and 100, got {v}"
        ))?;
        return Ok(None);
    }
    Ok(Some(FloatOrInt(v)))
}

fn parse_scroll_factor(val: &Value) -> Result<ScrollFactor, String> {
    match val {
        Value::Table(t) => {
            let base: Option<f64> = t.get("base").map_err(|e| format!("{e}")).ok();
            let h: Option<f64> = t.get("horizontal").map_err(|e| format!("{e}")).ok();
            let v: Option<f64> = t.get("vertical").map_err(|e| format!("{e}")).ok();
            Ok(ScrollFactor {
                base: base.map(FloatOrInt),
                horizontal: h.map(FloatOrInt),
                vertical: v.map(FloatOrInt),
            })
        }
        _ => {
            let v = value_to_f64(val)?;
            Ok(ScrollFactor {
                base: Some(FloatOrInt(v)),
                horizontal: None,
                vertical: None,
            })
        }
    }
}

/// Reads `table[snake]`, falling back to `table[kebab]` when the snake_case key is absent.
fn table_get_alias(table: &LuaTable, snake: &str, kebab: &str) -> Result<Value, String> {
    let v: Value = table.get(snake).map_err(|e| format!("{e}"))?;
    if matches!(v, Value::Nil) {
        table.get(kebab).map_err(|e| format!("{e}"))
    } else {
        Ok(v)
    }
}

fn parse_outputs(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for item in table.sequence_values::<LuaTable>() {
        let t = item.map_err(|e| format!("output error: {e}"))?;
        let name: String = t
            .get("name")
            .map_err(|e| format!("output needs name: {e}"))?;
        let off: bool = t.get("off").unwrap_or(false);
        let create_virtual =
            value_is_truthy(&table_get_alias(&t, "create_virtual", "create-virtual")?);
        let scale: Option<f64> = t.get("scale").map_err(|e| format!("{e}")).ok();
        let transform_str: Option<String> = t.get("transform").map_err(|e| format!("{e}")).ok();
        let focus_at_startup: bool = t.get("focus_at_startup").unwrap_or(false);

        let transform = match transform_str.as_deref() {
            Some("normal") | None => Transform::Normal,
            Some("90") => Transform::Flipped90,
            Some("180") => Transform::Flipped180,
            Some("270") => Transform::Flipped270,
            Some("flipped") => Transform::Flipped,
            Some("flipped-90") | Some("flipped_90") => Transform::Flipped90,
            Some("flipped-180") | Some("flipped_180") => Transform::Flipped180,
            Some("flipped-270") | Some("flipped_270") => Transform::Flipped270,
            _ => Transform::Normal,
        };

        let mode_str: Option<String> = t.get("mode").map_err(|e| format!("{e}")).ok();
        let mode = mode_str.as_deref().and_then(parse_mode_string);

        let position: Option<Position> = match t
            .get::<Option<Value>>("position")
            .map_err(|e| format!("output position error: {e}"))?
        {
            Some(Value::Table(pt)) => Some(Position {
                x: pt.get("x").map_err(|e| format!("position x error: {e}"))?,
                y: pt.get("y").map_err(|e| format!("position y error: {e}"))?,
            }),
            _ => None,
        };

        let max_bpc: Option<MaxBpc> = match table_get_alias(&t, "max_bpc", "max-bpc")? {
            Value::Integer(n) => Some(MaxBpc(
                neori_ipc::MaxBpc::try_from(u8::try_from(n).map_err(|e| format!("max-bpc: {e}"))?)
                    .map_err(|e| format!("max-bpc: {e}"))?,
            )),
            Value::String(s) => Some(MaxBpc(
                neori_ipc::MaxBpc::from_str(&s.to_str().map_err(|e| format!("{e}"))?)
                    .map_err(|e| format!("max-bpc: {e}"))?,
            )),
            _ => None,
        };

        let modeline: Option<Modeline> = match t
            .get::<Option<Value>>("modeline")
            .map_err(|e| format!("output modeline error: {e}"))?
        {
            Some(Value::Table(mt)) => Some(Modeline {
                clock: mt
                    .get("clock")
                    .map_err(|e| format!("modeline clock error: {e}"))?,
                hdisplay: mt
                    .get("hdisplay")
                    .map_err(|e| format!("modeline hdisplay error: {e}"))?,
                hsync_start: mt
                    .get("hsync_start")
                    .map_err(|e| format!("modeline hsync_start error: {e}"))?,
                hsync_end: mt
                    .get("hsync_end")
                    .map_err(|e| format!("modeline hsync_end error: {e}"))?,
                htotal: mt
                    .get("htotal")
                    .map_err(|e| format!("modeline htotal error: {e}"))?,
                vdisplay: mt
                    .get("vdisplay")
                    .map_err(|e| format!("modeline vdisplay error: {e}"))?,
                vsync_start: mt
                    .get("vsync_start")
                    .map_err(|e| format!("modeline vsync_start error: {e}"))?,
                vsync_end: mt
                    .get("vsync_end")
                    .map_err(|e| format!("modeline vsync_end error: {e}"))?,
                vtotal: mt
                    .get("vtotal")
                    .map_err(|e| format!("modeline vtotal error: {e}"))?,
                hsync_polarity: {
                    let s = value_to_string(&table_get_alias(
                        &mt,
                        "hsync_polarity",
                        "hsync-polarity",
                    )?)?;
                    neori_ipc::HSyncPolarity::from_str(&s)
                        .map_err(|e| format!("modeline hsync_polarity: {e}"))?
                },
                vsync_polarity: {
                    let s = value_to_string(&table_get_alias(
                        &mt,
                        "vsync_polarity",
                        "vsync-polarity",
                    )?)?;
                    neori_ipc::VSyncPolarity::from_str(&s)
                        .map_err(|e| format!("modeline vsync_polarity: {e}"))?
                },
            }),
            _ => None,
        };

        let variable_refresh_rate: Option<Vrr> =
            match table_get_alias(&t, "variable_refresh_rate", "variable-refresh-rate")? {
                Value::Boolean(true) => Some(Vrr { on_demand: false }),
                Value::Table(vt) => Some(Vrr {
                    on_demand: match table_get_alias(&vt, "on_demand", "on-demand")? {
                        Value::Nil => false,
                        v => value_is_truthy(&v),
                    },
                }),
                _ => None,
            };

        let background_color = match table_get_alias(&t, "background_color", "background-color")? {
            Value::String(s) => parse_css_color(&s.to_str().map_err(|e| format!("{e}"))?).ok(),
            _ => None,
        };

        let backdrop_color = match table_get_alias(&t, "backdrop_color", "backdrop-color")? {
            Value::String(s) => parse_css_color(&s.to_str().map_err(|e| format!("{e}"))?).ok(),
            _ => None,
        };

        let hot_corners: Option<HotCorners> =
            match table_get_alias(&t, "hot_corners", "hot-corners")? {
                Value::Table(hct) => Some(HotCorners {
                    off: hct.get("off").map_err(|e| format!("{e}")).unwrap_or(false),
                    top_left: value_is_truthy(&table_get_alias(&hct, "top_left", "top-left")?),
                    top_right: value_is_truthy(&table_get_alias(&hct, "top_right", "top-right")?),
                    bottom_left: value_is_truthy(&table_get_alias(
                        &hct,
                        "bottom_left",
                        "bottom-left",
                    )?),
                    bottom_right: value_is_truthy(&table_get_alias(
                        &hct,
                        "bottom_right",
                        "bottom-right",
                    )?),
                }),
                _ => None,
            };

        let layout_table: Option<LuaTable> = t
            .get("layout")
            .map_err(|e| format!("output layout error: {e}"))?;
        let layout = match layout_table {
            Some(lt) => Some(parse_layout_part(&lt)?),
            None => None,
        };

        config.outputs.0.push(Output {
            off,
            create_virtual,
            name,
            scale: scale.map(FloatOrInt),
            transform,
            position,
            max_bpc,
            mode,
            modeline,
            variable_refresh_rate,
            focus_at_startup,
            background_color,
            backdrop_color,
            hot_corners,
            layout,
        });
    }
    Ok(())
}

fn parse_mode_string(s: &str) -> Option<Mode> {
    let parts: Vec<&str> = s.split('@').collect();
    let res_parts: Vec<&str> = parts[0].split('x').collect();
    let width = res_parts.first()?.parse().ok()?;
    let height = res_parts.get(1)?.parse().ok()?;
    let refresh = parts.get(1).and_then(|s| s.parse::<f64>().ok());
    Some(Mode {
        custom: false,
        mode: ConfiguredMode {
            width,
            height,
            refresh,
        },
    })
}

fn parse_window_rules(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for item in table.sequence_values::<LuaTable>() {
        let t = item.map_err(|e| format!("window-rule error: {e}"))?;
        config.window_rules.push(parse_window_rule(t)?);
    }
    Ok(())
}

fn parse_window_rule(table: LuaTable) -> Result<WindowRule, String> {
    let mut rule = WindowRule::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("window-rule field error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "match" | "matches" => {
                if let Value::Table(t) = &value {
                    let mut has_seq = false;
                    for v in t.clone().sequence_values::<LuaTable>() {
                        has_seq = true;
                        let mt = v.map_err(|e| format!("{e}"))?;
                        rule.matches.push(parse_match_entry(&mt)?);
                    }
                    if !has_seq {
                        rule.matches.push(parse_match_entry(t)?);
                    }
                }
            }
            "exclude" | "excludes" => {
                if let Value::Table(t) = &value {
                    let mut has_seq = false;
                    for v in t.clone().sequence_values::<LuaTable>() {
                        has_seq = true;
                        let mt = v.map_err(|e| format!("{e}"))?;
                        rule.excludes.push(parse_match_entry(&mt)?);
                    }
                    if !has_seq {
                        rule.excludes.push(parse_match_entry(t)?);
                    }
                }
            }
            "open_on_output" | "open-on-output" => {
                rule.open_on_output = Some(value_to_string(&value)?);
            }
            "open_on_workspace" | "open-on-workspace" => {
                rule.open_on_workspace = Some(value_to_string(&value)?);
            }
            "open_maximized" | "open-maximized" => {
                rule.open_maximized = Some(value_is_truthy(&value));
            }
            "open_maximized_to_edges" | "open-maximized-to-edges" => {
                rule.open_maximized_to_edges = Some(value_is_truthy(&value));
            }
            "open_fullscreen" | "open-fullscreen" => {
                rule.open_fullscreen = Some(value_is_truthy(&value));
            }
            "open_floating" | "open-floating" => {
                rule.open_floating = Some(value_is_truthy(&value));
            }
            "open_focused" | "open-focused" => {
                rule.open_focused = Some(value_is_truthy(&value));
            }
            "on_xdg_activate" | "on-xdg-activate" => {
                let s = value_to_string(&value)?;
                rule.on_xdg_activate = match s.as_str() {
                    "ignore" => Some(OnXdgActivate::Ignore),
                    "set-urgent" | "set_urgent" => Some(OnXdgActivate::SetUrgent),
                    "focus" => Some(OnXdgActivate::Focus),
                    other => {
                        invalid_config(&format!(
                            "invalid on_xdg_activate `{other}`, expected \"ignore\", \"set-urgent\" or \"focus\""
                        ))?;
                        None
                    }
                };
            }
            "min_width" | "min-width" => {
                rule.min_width = Some(value_to_u16(&value)?);
            }
            "min_height" | "min-height" => {
                rule.min_height = Some(value_to_u16(&value)?);
            }
            "max_width" | "max-width" => {
                rule.max_width = Some(value_to_u16(&value)?);
            }
            "max_height" | "max-height" => {
                rule.max_height = Some(value_to_u16(&value)?);
            }
            "default_column_width" | "default-column-width" => {
                if let Value::Table(_) = &value {
                    rule.default_column_width = Some(value_to_default_preset_size(&value)?);
                }
            }
            "default_window_height" | "default-window-height" => {
                if let Value::Table(_) = &value {
                    rule.default_window_height = Some(value_to_default_preset_size(&value)?);
                }
            }
            "default_column_display" | "default-column-display" => {
                let s = value_to_string(&value)?;
                rule.default_column_display = Some(
                    s.parse::<ColumnDisplay>()
                        .map_err(|e| format!("invalid default-column-display: {e}"))?,
                );
            }
            "focus_ring" | "focus-ring" => {
                if let Value::Table(t) = &value {
                    rule.focus_ring = parse_border_rule(t)?;
                }
            }
            "border" => {
                if let Value::Table(t) = &value {
                    rule.border = parse_border_rule(t)?;
                }
            }
            "shadow" => {
                if let Value::Table(t) = &value {
                    rule.shadow = parse_shadow_rule(t)?;
                }
            }
            "draw_border_with_background" | "draw-border-with-background" => {
                rule.draw_border_with_background = Some(value_is_truthy(&value));
            }
            "opacity" => {
                rule.opacity = Some(value_to_f64(&value)? as f32);
            }
            "pinch_sensitivity" | "pinch-sensitivity" => {
                rule.pinch_sensitivity = parse_pinch_sensitivity(&value, "window_rule")?;
            }
            "geometry_corner_radius" | "geometry-corner-radius" => {
                rule.geometry_corner_radius = Some(value_to_corner_radius(&value)?);
            }
            "clip_to_geometry" | "clip-to-geometry" => {
                rule.clip_to_geometry = Some(value_is_truthy(&value));
            }
            "baba_is_float" | "baba-is-float" => {
                rule.baba_is_float = Some(value_is_truthy(&value));
            }
            "variable_refresh_rate" | "variable-refresh-rate" => {
                rule.variable_refresh_rate = match &value {
                    Value::Boolean(true) => Some(VrrRule::On),
                    Value::Boolean(false) => Some(VrrRule::Off),
                    _ => match value_to_string(&value)?.as_str() {
                        // Also accept bool strings to reduce confusion.
                        "true" => Some(VrrRule::On),
                        "false" => Some(VrrRule::Off),
                        "fullscreen-only" | "fullscreen_only" => Some(VrrRule::FullscreenOnly),
                        other => {
                            invalid_config(&format!(
                                "invalid variable_refresh_rate `{other}`, expected true, \"fullscreen-only\", or false"
                            ))?;
                            None
                        }
                    },
                };
            }
            "tiled_state" | "tiled-state" => {
                rule.tiled_state = Some(value_is_truthy(&value));
            }
            _ => unknown_key("window_rule", &ks)?,
        }
    }
    Ok(rule)
}

fn value_to_corner_radius(val: &Value) -> Result<CornerRadius, String> {
    match val {
        Value::Integer(_) | Value::Number(_) | Value::String(_) => {
            Ok(CornerRadius::from(value_to_f64(val)? as f32))
        }
        Value::Table(t) => {
            let named = |key: &str| t.get::<Option<f64>>(key).ok().flatten();
            let top_left = named("top_left").or_else(|| named("top-left"));
            let top_right = named("top_right").or_else(|| named("top-right"));
            let bottom_right = named("bottom_right").or_else(|| named("bottom-right"));
            let bottom_left = named("bottom_left").or_else(|| named("bottom-left"));
            if top_left.is_some()
                || top_right.is_some()
                || bottom_right.is_some()
                || bottom_left.is_some()
            {
                return Ok(CornerRadius {
                    top_left: top_left.unwrap_or(0.0) as f32,
                    top_right: top_right.unwrap_or(0.0) as f32,
                    bottom_right: bottom_right.unwrap_or(0.0) as f32,
                    bottom_left: bottom_left.unwrap_or(0.0) as f32,
                });
            }
            let values: Vec<f64> = t.clone().sequence_values::<f64>().flatten().collect();
            match values.as_slice() {
                [all] => Ok(CornerRadius::from(*all as f32)),
                [tl, tr, br, bl] => Ok(CornerRadius {
                    top_left: *tl as f32,
                    top_right: *tr as f32,
                    bottom_right: *br as f32,
                    bottom_left: *bl as f32,
                }),
                _ => Err(format!(
                    "expected a number or four corner radii, got {} values",
                    values.len()
                )),
            }
        }
        _ => Err(format!(
            "expected a number or table for corner radius, got {:?}",
            val.type_name()
        )),
    }
}

fn str_to_regex(s: &str) -> Option<RegexEq> {
    std::str::FromStr::from_str(s).ok().map(RegexEq)
}

fn parse_match_entry(table: &LuaTable) -> Result<Match, String> {
    let app_id: Option<String> = table
        .get("app_id")
        .map_err(|e| format!("{e}"))
        .ok()
        .or_else(|| {
            let v: Result<String, _> = table.get("app-id");
            v.ok()
        });
    let title: Option<String> = table.get("title").map_err(|e| format!("{e}")).ok();

    Ok(Match {
        app_id: app_id.as_deref().and_then(str_to_regex),
        title: title.as_deref().and_then(str_to_regex),
        is_active: table.get("is_active").unwrap_or(None),
        is_focused: table.get("is_focused").unwrap_or(None),
        is_active_in_column: table.get("is_active_in_column").unwrap_or(None),
        is_floating: table.get("is_floating").unwrap_or(None),
        is_window_cast_target: table.get("is_window_cast_target").unwrap_or(None),
        is_urgent: table.get("is_urgent").unwrap_or(None),
        at_startup: table.get("at_startup").unwrap_or(None),
    })
}

fn parse_layer_rules(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for item in table.sequence_values::<LuaTable>() {
        let t = item.map_err(|e| format!("layer-rule error: {e}"))?;
        config.layer_rules.push(parse_layer_rule(t)?);
    }
    Ok(())
}

fn parse_layer_rule(table: LuaTable) -> Result<LayerRule, String> {
    let mut rule = LayerRule::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("layer-rule error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "match" | "matches" => {
                if let Value::Table(t) = &value {
                    let mut has_seq = false;
                    for v in t.clone().sequence_values::<LuaTable>() {
                        has_seq = true;
                        let mt = v.map_err(|e| format!("{e}"))?;
                        rule.matches.push(parse_layer_match_entry(&mt)?);
                    }
                    if !has_seq {
                        rule.matches.push(parse_layer_match_entry(t)?);
                    }
                }
            }
            "block_out_from" | "block-out-from" => {
                let s = value_to_string(&value)?;
                rule.block_out_from = match s.as_str() {
                    "screencast" => Some(BlockOutFrom::Screencast),
                    "screen-capture" | "screen_capture" => Some(BlockOutFrom::ScreenCapture),
                    _ => None,
                };
            }
            _ => unknown_key("layer_rule", &ks)?,
        }
    }
    Ok(rule)
}

fn parse_layer_match_entry(table: &LuaTable) -> Result<crate::layer_rule::Match, String> {
    let ns: Option<String> = table.get("namespace").map_err(|e| format!("{e}")).ok();
    let layer_str: Option<String> = table.get("layer").map_err(|e| format!("{e}")).ok();
    let layer = layer_str.as_deref().and_then(|s| match s {
        "background" | "Background" => Some(Layer::Background),
        "bottom" | "Bottom" => Some(Layer::Bottom),
        "top" | "Top" => Some(Layer::Top),
        "overlay" | "Overlay" => Some(Layer::Overlay),
        _ => None,
    });
    Ok(crate::layer_rule::Match {
        namespace: ns.as_deref().and_then(str_to_regex),
        at_startup: table
            .get("at_startup")
            .map_err(|e| format!("{e}"))
            .ok()
            .flatten(),
        layer,
    })
}

/// Parses a named workspace's `open_on_output`: a single output name, or a list of output
/// names in preference order.
fn parse_open_on_output(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::String(_) => Ok(vec![value_to_string(value)?]),
        Value::Table(t) => t
            .clone()
            .sequence_values::<Value>()
            .map(|v| value_to_string(&v.map_err(|e| format!("open_on_output: {e}"))?))
            .collect(),
        other => Err(format!(
            "open_on_output must be a string or a list of strings, got {}",
            other.type_name()
        )),
    }
}

fn parse_workspaces(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for item in table.sequence_values::<LuaTable>() {
        let t = item.map_err(|e| format!("workspace error: {e}"))?;
        let name: String = t.get("name").unwrap_or_default();
        let open_on_output = parse_open_on_output(&table_get_alias(
            &t,
            "open_on_output",
            "open-on-output",
        )?)?;
        let layout_table: Option<LuaTable> = t
            .get("layout")
            .map_err(|e| format!("workspace layout error: {e}"))?;
        let layout = match layout_table {
            Some(lt) => Some(WorkspaceLayoutPart(parse_layout_part(&lt)?)),
            None => None,
        };
        config.workspaces.push(Workspace {
            name: WorkspaceName(name),
            open_on_output,
            layout,
        });
    }
    Ok(())
}

fn parse_scratchpads(table: LuaTable, config: &mut Config) -> Result<(), String> {
    // A single map-style entry (`scratchpads = { name = "term", ... }`) is accepted alongside
    // the usual sequence of entries.
    let entries: Vec<Value> = if table_is_map(&table)? {
        vec![Value::Table(table)]
    } else {
        table
            .clone()
            .sequence_values::<Value>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("scratchpad error: {e}"))?
    };

    for item in entries {
        // Bare name: `neori.scratchpad("term")`.
        let (name, spawn, spawn_sh) = match item {
            Value::String(s) => (
                s.to_str().map_err(|e| format!("{e}"))?.to_string(),
                None,
                None,
            ),
            Value::Table(t) => {
                let name: String = t.get("name").unwrap_or_default();
                let spawn = t.get::<Value>("spawn").map_err(|e| format!("{e}"))?;
                let spawn = match spawn {
                    Value::Nil => None,
                    v => {
                        let args = parse_spawn_args(&v)?;
                        if args.is_empty() {
                            return Err(format!("scratchpad `{name}`: spawn requires a command"));
                        }
                        Some(args)
                    }
                };
                let spawn_sh = match table_get_alias(&t, "spawn_sh", "spawn-sh")? {
                    Value::Nil => None,
                    Value::String(s) => Some(s.to_str().map_err(|e| format!("{e}"))?.to_string()),
                    _ => {
                        return Err(format!("scratchpad `{name}`: spawn_sh must be a string"));
                    }
                };
                if spawn.is_some() && spawn_sh.is_some() {
                    return Err(format!(
                        "scratchpad `{name}`: set only one of `spawn` or `spawn_sh`"
                    ));
                }
                (name, spawn, spawn_sh)
            }
            _ => {
                return Err(
                    "scratchpad entries expect a name string or a table of options".to_string(),
                );
            }
        };

        if config.scratchpads.iter().any(|s| s.name == name) {
            return Err(format!("duplicate scratchpad name `{name}`"));
        }
        config.scratchpads.push(Scratchpad {
            name,
            spawn,
            spawn_sh,
        });
    }
    Ok(())
}

fn parse_layout_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("layout error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "gaps" => {
                if let Ok(v) = value_to_f64(&value) {
                    config.layout.gaps = v;
                }
            }
            "focus_ring" | "focus-ring" => {
                if let Value::Table(t) = &value {
                    parse_focus_ring(t, &mut config.layout.focus_ring)?;
                }
            }
            "border" => {
                if let Value::Table(t) = &value {
                    parse_border(t, &mut config.layout.border)?;
                }
            }
            "shadow" => {
                if let Value::Table(t) = &value {
                    parse_shadow(t, &mut config.layout.shadow)?;
                }
            }
            "tab_indicator" | "tab-indicator" => {
                if let Value::Table(t) = &value {
                    parse_tab_indicator(t, &mut config.layout.tab_indicator)?;
                }
            }
            "insert_hint" | "insert-hint" => {
                if let Value::Table(t) = &value {
                    parse_insert_hint(t, &mut config.layout.insert_hint)?;
                }
            }
            "preset_column_widths" | "preset-column-widths" => {
                if let Value::Table(t) = &value {
                    let mut sizes = Vec::new();
                    for item in t.clone().sequence_values::<LuaTable>() {
                        sizes.push(parse_preset_size(item.map_err(|e| format!("{e}"))?)?);
                    }
                    config.layout.preset_column_widths = sizes;
                }
            }
            "preset_window_heights" | "preset-window-heights" => {
                if let Value::Table(t) = &value {
                    let mut sizes = Vec::new();
                    for item in t.clone().sequence_values::<LuaTable>() {
                        sizes.push(parse_preset_size(item.map_err(|e| format!("{e}"))?)?);
                    }
                    config.layout.preset_window_heights = sizes;
                }
            }
            "default_column_width" | "default-column-width" => {
                if let Value::Table(t) = &value {
                    config.layout.default_column_width = parse_preset_opt(t)?;
                } else if matches!(value, Value::Nil) {
                    config.layout.default_column_width = None;
                }
            }
            "center_focused_column" | "center-focused-column" => {
                let s = value_to_string(&value)?;
                config.layout.center_focused_column = match s.as_str() {
                    "never" => CenterFocusedColumn::Never,
                    "always" => CenterFocusedColumn::Always,
                    "on-overflow" | "on_overflow" => CenterFocusedColumn::OnOverflow,
                    _ => return Err(format!("invalid center-focused-column: {s}")),
                };
            }
            "always_center_single_column" | "always-center-single-column" => {
                config.layout.always_center_single_column = value_is_truthy(&value);
            }
            "empty_workspace_above_first" | "empty-workspace-above-first" => {
                config.layout.empty_workspace_above_first = value_is_truthy(&value);
            }
            "default_column_display" | "default-column-display" => {
                let s = value_to_string(&value)?;
                config.layout.default_column_display = match s.as_str() {
                    "normal" => ColumnDisplay::Normal,
                    "tabbed" => ColumnDisplay::Tabbed,
                    _ => return Err(format!("invalid default-column-display: {s}")),
                };
            }
            "struts" => {
                if let Value::Table(t) = &value {
                    config.layout.struts.left = FloatOrInt(t.get("left").unwrap_or(0.0f64));
                    config.layout.struts.right = FloatOrInt(t.get("right").unwrap_or(0.0f64));
                    config.layout.struts.top = FloatOrInt(t.get("top").unwrap_or(0.0f64));
                    config.layout.struts.bottom = FloatOrInt(t.get("bottom").unwrap_or(0.0f64));
                }
            }
            "background_color" | "background-color" => {
                let s = value_to_string(&value)?;
                config.layout.background_color = parse_css_color(&s)?;
            }
            "fullscreen_backdrop_color" | "fullscreen-backdrop-color" => {
                let s = value_to_string(&value)?;
                config.layout.fullscreen_backdrop_color = parse_css_color(&s)?;
            }
            _ => unknown_key("layout", &ks)?,
        }
    }
    Ok(())
}

fn parse_focus_ring(table: &LuaTable, ring: &mut FocusRing) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("focus-ring error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => ring.off = value_is_truthy(&value),
            "width" => ring.width = value_to_f64(&value)?,
            "active_color" | "active-color" => {
                ring.active_color = parse_css_color(&value_to_string(&value)?)?;
            }
            "inactive_color" | "inactive-color" => {
                ring.inactive_color = parse_css_color(&value_to_string(&value)?)?;
            }
            "urgent_color" | "urgent-color" => {
                ring.urgent_color = parse_css_color(&value_to_string(&value)?)?;
            }
            _ => unknown_key("layout.focus_ring", &ks)?,
        }
    }
    Ok(())
}

fn parse_border(table: &LuaTable, border: &mut Border) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("border error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => {
                if value_is_truthy(&value) {
                    border.off = true;
                }
            }
            "width" => border.width = value_to_f64(&value)?,
            "active_color" | "active-color" => {
                border.active_color = parse_css_color(&value_to_string(&value)?)?;
            }
            "inactive_color" | "inactive-color" => {
                border.inactive_color = parse_css_color(&value_to_string(&value)?)?;
            }
            "urgent_color" | "urgent-color" => {
                border.urgent_color = parse_css_color(&value_to_string(&value)?)?;
            }
            _ => unknown_key("layout.border", &ks)?,
        }
    }
    Ok(())
}

fn parse_shadow(table: &LuaTable, shadow: &mut Shadow) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("shadow error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "on" => shadow.on = value_is_truthy(&value),
            "off" => {
                if value_is_truthy(&value) {
                    shadow.on = false;
                }
            }
            "softness" => shadow.softness = value_to_f64(&value)?,
            "spread" => shadow.spread = value_to_f64(&value)?,
            "draw_behind_window" | "draw-behind-window" => {
                shadow.draw_behind_window = value_is_truthy(&value);
            }
            "color" => {
                shadow.color = parse_css_color(&value_to_string(&value)?)?;
            }
            "inactive_color" | "inactive-color" => {
                shadow.inactive_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "offset" => {
                if let Value::Table(t) = &value {
                    shadow.offset.x = FloatOrInt(t.get("x").unwrap_or(0.0f64));
                    shadow.offset.y = FloatOrInt(t.get("y").unwrap_or(0.0f64));
                }
            }
            _ => unknown_key("layout.shadow", &ks)?,
        }
    }
    Ok(())
}

fn parse_tab_indicator(table: &LuaTable, tab: &mut TabIndicator) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("tab-indicator error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => tab.off = value_is_truthy(&value),
            "hide_when_single_tab" | "hide-when-single-tab" => {
                tab.hide_when_single_tab = value_is_truthy(&value);
            }
            "width" => tab.width = value_to_f64(&value)?,
            "gap" => tab.gap = value_to_f64(&value)?,
            "gaps_between_tabs" | "gaps-between-tabs" => {
                tab.gaps_between_tabs = value_to_f64(&value)?;
            }
            "position" => {
                let s = value_to_string(&value)?;
                tab.position = match s.as_str() {
                    "left" => TabIndicatorPosition::Left,
                    "right" => TabIndicatorPosition::Right,
                    "top" => TabIndicatorPosition::Top,
                    "bottom" => TabIndicatorPosition::Bottom,
                    _ => return Err(format!("invalid tab-indicator position: {s}")),
                };
            }
            "corner_radius" | "corner-radius" => {
                tab.corner_radius = value_to_f64(&value)?;
            }
            "active_color" | "active-color" => {
                if let Ok(c) = parse_css_color(&value_to_string(&value)?) {
                    tab.active_color = Some(c);
                }
            }
            "inactive_color" | "inactive-color" => {
                if let Ok(c) = parse_css_color(&value_to_string(&value)?) {
                    tab.inactive_color = Some(c);
                }
            }
            "urgent_color" | "urgent-color" => {
                if let Ok(c) = parse_css_color(&value_to_string(&value)?) {
                    tab.urgent_color = Some(c);
                }
            }
            "length" => {
                if let Value::Table(t) = &value {
                    tab.length.total_proportion =
                        t.get("total_proportion").map_err(|e| format!("{e}")).ok();
                } else if let Ok(v) = value_to_f64(&value) {
                    tab.length.total_proportion = Some(v);
                }
            }
            _ => unknown_key("layout.tab_indicator", &ks)?,
        }
    }
    Ok(())
}

fn parse_insert_hint(table: &LuaTable, hint: &mut InsertHint) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("insert-hint error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => hint.off = value_is_truthy(&value),
            "color" => hint.color = parse_css_color(&value_to_string(&value)?)?,
            _ => unknown_key("layout.insert_hint", &ks)?,
        }
    }
    Ok(())
}

/// Parses a `LayoutPart`, the partial layout overrides used by outputs and workspaces.
fn parse_layout_part(table: &LuaTable) -> Result<LayoutPart, String> {
    let mut part = LayoutPart::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("layout error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "focus_ring" | "focus-ring" => {
                if let Value::Table(t) = &value {
                    part.focus_ring = Some(parse_border_rule(t)?);
                }
            }
            "border" => {
                if let Value::Table(t) = &value {
                    part.border = Some(parse_border_rule(t)?);
                }
            }
            "shadow" => {
                if let Value::Table(t) = &value {
                    part.shadow = Some(parse_shadow_rule(t)?);
                }
            }
            "tab_indicator" | "tab-indicator" => {
                if let Value::Table(t) = &value {
                    part.tab_indicator = Some(parse_tab_indicator_part(t)?);
                }
            }
            "insert_hint" | "insert-hint" => {
                if let Value::Table(t) = &value {
                    part.insert_hint = Some(parse_insert_hint_part(t)?);
                }
            }
            "preset_column_widths" | "preset-column-widths" => {
                if let Value::Table(t) = &value {
                    let mut sizes = Vec::new();
                    for item in t.clone().sequence_values::<LuaTable>() {
                        sizes.push(parse_preset_size(item.map_err(|e| format!("{e}"))?)?);
                    }
                    part.preset_column_widths = Some(sizes);
                }
            }
            "default_column_width" | "default-column-width" => {
                part.default_column_width = Some(value_to_default_preset_size(&value)?);
            }
            "preset_window_heights" | "preset-window-heights" => {
                if let Value::Table(t) = &value {
                    let mut sizes = Vec::new();
                    for item in t.clone().sequence_values::<LuaTable>() {
                        sizes.push(parse_preset_size(item.map_err(|e| format!("{e}"))?)?);
                    }
                    part.preset_window_heights = Some(sizes);
                }
            }
            "center_focused_column" | "center-focused-column" => {
                let s = value_to_string(&value)?;
                part.center_focused_column = Some(match s.as_str() {
                    "never" => CenterFocusedColumn::Never,
                    "always" => CenterFocusedColumn::Always,
                    "on-overflow" | "on_overflow" => CenterFocusedColumn::OnOverflow,
                    _ => return Err(format!("invalid center-focused-column: {s}")),
                });
            }
            "always_center_single_column" | "always-center-single-column" => {
                part.always_center_single_column = Some(value_is_truthy(&value));
            }
            "empty_workspace_above_first" | "empty-workspace-above-first" => {
                part.empty_workspace_above_first = Some(value_is_truthy(&value));
            }
            "default_column_display" | "default-column-display" => {
                let s = value_to_string(&value)?;
                part.default_column_display = Some(
                    s.parse::<ColumnDisplay>()
                        .map_err(|e| format!("invalid default-column-display: {e}"))?,
                );
            }
            "gaps" => {
                part.gaps = Some(FloatOrInt(value_to_f64(&value)?));
            }
            "struts" => {
                if let Value::Table(t) = &value {
                    part.struts = Some(Struts {
                        left: FloatOrInt(t.get("left").unwrap_or(0.0f64)),
                        right: FloatOrInt(t.get("right").unwrap_or(0.0f64)),
                        top: FloatOrInt(t.get("top").unwrap_or(0.0f64)),
                        bottom: FloatOrInt(t.get("bottom").unwrap_or(0.0f64)),
                    });
                }
            }
            "background_color" | "background-color" => {
                let s = value_to_string(&value)?;
                part.background_color = Some(parse_css_color(&s)?);
            }
            "fullscreen_backdrop_color" | "fullscreen-backdrop-color" => {
                let s = value_to_string(&value)?;
                part.fullscreen_backdrop_color = Some(parse_css_color(&s)?);
            }
            _ => unknown_key("layout part", &ks)?,
        }
    }
    Ok(part)
}

fn parse_border_rule(table: &LuaTable) -> Result<BorderRule, String> {
    let mut rule = BorderRule::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("border error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => rule.off = value_is_truthy(&value),
            "on" => rule.on = value_is_truthy(&value),
            "width" => rule.width = Some(FloatOrInt(value_to_f64(&value)?)),
            "active_color" | "active-color" => {
                rule.active_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "inactive_color" | "inactive-color" => {
                rule.inactive_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "urgent_color" | "urgent-color" => {
                rule.urgent_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            _ => unknown_key("border rule", &ks)?,
        }
    }
    Ok(rule)
}

fn parse_shadow_rule(table: &LuaTable) -> Result<ShadowRule, String> {
    let mut rule = ShadowRule::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("shadow error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => rule.off = value_is_truthy(&value),
            "on" => rule.on = value_is_truthy(&value),
            "offset" => {
                if let Value::Table(t) = &value {
                    let x: f64 = t.get("x").map_err(|e| format!("{e}")).unwrap_or(0.0);
                    let y: f64 = t.get("y").map_err(|e| format!("{e}")).unwrap_or(0.0);
                    rule.offset = Some(ShadowOffset {
                        x: FloatOrInt(x),
                        y: FloatOrInt(y),
                    });
                }
            }
            "softness" => rule.softness = Some(FloatOrInt(value_to_f64(&value)?)),
            "spread" => rule.spread = Some(FloatOrInt(value_to_f64(&value)?)),
            "draw_behind_window" | "draw-behind-window" => {
                rule.draw_behind_window = Some(value_is_truthy(&value));
            }
            "color" => {
                rule.color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "inactive_color" | "inactive-color" => {
                rule.inactive_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            _ => unknown_key("shadow rule", &ks)?,
        }
    }
    Ok(rule)
}

fn parse_tab_indicator_part(table: &LuaTable) -> Result<TabIndicatorPart, String> {
    let mut part = TabIndicatorPart::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("tab-indicator error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => part.off = value_is_truthy(&value),
            "on" => part.on = value_is_truthy(&value),
            "hide_when_single_tab" | "hide-when-single-tab" => {
                part.hide_when_single_tab = Some(Flag(value_is_truthy(&value)));
            }
            "place_within_column" | "place-within-column" => {
                part.place_within_column = Some(Flag(value_is_truthy(&value)));
            }
            "gap" => part.gap = Some(FloatOrInt(value_to_f64(&value)?)),
            "width" => part.width = Some(FloatOrInt(value_to_f64(&value)?)),
            "length" => {
                let len = match &value {
                    Value::Table(t) => TabIndicatorLength {
                        total_proportion: t
                            .get("total_proportion")
                            .map_err(|e| format!("{e}"))
                            .ok(),
                    },
                    _ => TabIndicatorLength {
                        total_proportion: Some(value_to_f64(&value)?),
                    },
                };
                part.length = Some(len);
            }
            "position" => {
                let s = value_to_string(&value)?;
                part.position = Some(match s.as_str() {
                    "left" => TabIndicatorPosition::Left,
                    "right" => TabIndicatorPosition::Right,
                    "top" => TabIndicatorPosition::Top,
                    "bottom" => TabIndicatorPosition::Bottom,
                    _ => return Err(format!("invalid tab-indicator position: {s}")),
                });
            }
            "gaps_between_tabs" | "gaps-between-tabs" => {
                part.gaps_between_tabs = Some(FloatOrInt(value_to_f64(&value)?));
            }
            "corner_radius" | "corner-radius" => {
                part.corner_radius = Some(FloatOrInt(value_to_f64(&value)?));
            }
            "active_color" | "active-color" => {
                part.active_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "inactive_color" | "inactive-color" => {
                part.inactive_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            "urgent_color" | "urgent-color" => {
                part.urgent_color = Some(parse_css_color(&value_to_string(&value)?)?);
            }
            _ => unknown_key("tab_indicator rule", &ks)?,
        }
    }
    Ok(part)
}

fn parse_insert_hint_part(table: &LuaTable) -> Result<InsertHintPart, String> {
    let mut part = InsertHintPart::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("insert-hint error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => part.off = value_is_truthy(&value),
            "on" => part.on = value_is_truthy(&value),
            "color" => part.color = Some(parse_css_color(&value_to_string(&value)?)?),
            _ => unknown_key("insert_hint rule", &ks)?,
        }
    }
    Ok(part)
}

fn parse_preset_size(table: LuaTable) -> Result<PresetSize, String> {
    if let Ok(v) = table.get::<f64>("proportion") {
        Ok(PresetSize::Proportion(v))
    } else if let Ok(v) = table.get::<i64>("fixed") {
        Ok(PresetSize::Fixed(v as i32))
    } else {
        Err("preset size must have 'proportion' or 'fixed' field".to_string())
    }
}

fn parse_preset_opt(table: &LuaTable) -> Result<Option<PresetSize>, String> {
    if let Ok(v) = table.get::<f64>("proportion") {
        Ok(Some(PresetSize::Proportion(v)))
    } else if let Ok(v) = table.get::<i64>("fixed") {
        Ok(Some(PresetSize::Fixed(v as i32)))
    } else {
        Ok(None)
    }
}

fn parse_cursor_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("cursor error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "xcursor_theme" | "xcursor-theme" => {
                config.cursor.xcursor_theme = value_to_string(&value)?;
            }
            "xcursor_size" | "xcursor-size" => {
                config.cursor.xcursor_size = value_to_u8(&value)?;
            }
            "hide_when_typing" | "hide-when-typing" => {
                config.cursor.hide_when_typing = value_is_truthy(&value);
            }
            "hide_after_inactive_ms" | "hide-after-inactive-ms" => {
                config.cursor.hide_after_inactive_ms = Some(value_to_u32(&value)?);
            }
            "shake_to_find" | "shake-to-find" => match value {
                Value::Table(t) => parse_shake_to_find_table(t, &mut config.cursor.shake_to_find)?,
                _ => config.cursor.shake_to_find.enable = value_is_truthy(&value),
            },
            _ => unknown_key("cursor", &ks)?,
        }
    }
    Ok(())
}

fn parse_shake_to_find_table(table: LuaTable, shake: &mut ShakeToFind) -> Result<(), String> {
    // A table implies the feature is wanted unless `enable = false` is given.
    shake.enable = true;

    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("cursor.shake_to_find error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "enable" | "enabled" => shake.enable = value_is_truthy(&value),
            "size_multiplier" | "size-multiplier" => {
                let v = value_to_f64(&value)?;
                if !(v.is_finite() && v > 0.) {
                    return Err(format!(
                        "cursor.shake_to_find.size_multiplier must be a positive number, got {v}"
                    ));
                }
                shake.size_multiplier = v;
            }
            "size" => shake.size = Some(value_to_u8(&value)?),
            "timeout_ms" | "timeout-ms" => shake.timeout_ms = value_to_u32(&value)?,
            "animation_ms" | "animation-ms" => shake.animation_ms = value_to_u32(&value)?,
            "window_ms" | "window-ms" => {
                let v = value_to_u32(&value)?;
                if v == 0 {
                    return Err(
                        "cursor.shake_to_find.window_ms must be greater than zero".to_string()
                    );
                }
                shake.window_ms = v;
            }
            "min_reversals" | "min-reversals" => {
                let v = value_to_u32(&value)?;
                if v == 0 {
                    return Err(
                        "cursor.shake_to_find.min_reversals must be greater than zero".to_string(),
                    );
                }
                shake.min_reversals = v;
            }
            "min_step" | "min-step" => {
                let v = value_to_f64(&value)?;
                if !(v.is_finite() && v >= 0.) {
                    return Err(format!(
                        "cursor.shake_to_find.min_step must be a non-negative number, got {v}"
                    ));
                }
                shake.min_step = v;
            }
            "reversal_angle_deg" | "reversal-angle-deg" => {
                let v = value_to_f64(&value)?;
                if !(v.is_finite() && v > 0. && v < 180.) {
                    return Err(format!(
                        "cursor.shake_to_find.reversal_angle_deg must be between 0 and 180, got {v}"
                    ));
                }
                shake.reversal_angle_deg = v;
            }
            _ => unknown_key("cursor.shake_to_find", &ks)?,
        }
    }

    Ok(())
}

fn parse_hotkey_overlay_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("hotkey-overlay error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "skip_at_startup" | "skip-at-startup" => {
                config.hotkey_overlay.skip_at_startup = value_is_truthy(&value);
            }
            "hide_not_bound" | "hide-not-bound" => {
                config.hotkey_overlay.hide_not_bound = value_is_truthy(&value);
            }
            _ => unknown_key("hotkey_overlay", &ks)?,
        }
    }
    Ok(())
}

fn parse_config_notification_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("config-notification error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "disable_failed" | "disable-failed" => {
                config.config_notification.disable_failed = value_is_truthy(&value);
            }
            _ => unknown_key("config_notification", &ks)?,
        }
    }
    Ok(())
}

fn parse_clipboard_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("clipboard error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "disable_primary" | "disable-primary" => {
                config.clipboard.disable_primary = value_is_truthy(&value);
            }
            _ => unknown_key("clipboard", &ks)?,
        }
    }
    Ok(())
}

fn parse_blur_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("blur error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => config.blur.off = value_is_truthy(&value),
            "passes" => {
                if let Ok(v) = value_to_u32(&value) {
                    config.blur.passes = v as u8;
                }
            }
            "offset" => config.blur.offset = value_to_f64(&value)?,
            "noise" => config.blur.noise = value_to_f64(&value)?,
            "saturation" => config.blur.saturation = value_to_f64(&value)?,
            _ => unknown_key("blur", &ks)?,
        }
    }
    Ok(())
}

fn parse_gestures_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let gestures = &mut config.gestures;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("gestures error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "dnd_edge_view_scroll" | "dnd-edge-view-scroll" => {
                if let Value::Table(t) = &value {
                    gestures.dnd_edge_view_scroll.trigger_width = t
                        .get("trigger_width")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(10.0);
                    gestures.dnd_edge_view_scroll.delay_ms = t
                        .get("delay_ms")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(100u16);
                    gestures.dnd_edge_view_scroll.max_speed = t
                        .get("max_speed")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(50.0);
                }
            }
            "dnd_edge_workspace_switch" | "dnd-edge-workspace-switch" => {
                if let Value::Table(t) = &value {
                    gestures.dnd_edge_workspace_switch.trigger_height = t
                        .get("trigger_height")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(50.0);
                    gestures.dnd_edge_workspace_switch.delay_ms = t
                        .get("delay_ms")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(100u16);
                    gestures.dnd_edge_workspace_switch.max_speed = t
                        .get("max_speed")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(1500.0);
                }
            }
            "hot_corners" | "hot-corners" => {
                if let Value::Table(t) = &value {
                    gestures.hot_corners.off =
                        t.get("off").map_err(|e| format!("{e}")).unwrap_or(false);
                    gestures.hot_corners.top_left = t
                        .get("top_left")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(false);
                    gestures.hot_corners.top_right = t
                        .get("top_right")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(false);
                    gestures.hot_corners.bottom_left = t
                        .get("bottom_left")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(false);
                    gestures.hot_corners.bottom_right = t
                        .get("bottom_right")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(false);
                }
            }
            _ => unknown_key("gestures", &ks)?,
        }
    }
    Ok(())
}

fn parse_overview_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let overview = &mut config.overview;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("overview error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "zoom" => overview.zoom = value_to_f64(&value)?,
            "backdrop_color" | "backdrop-color" => {
                overview.backdrop_color = parse_css_color(&value_to_string(&value)?)?;
            }
            "workspace_shadow" | "workspace-shadow" => {
                if let Value::Table(t) = &value {
                    parse_workspace_shadow(t, &mut overview.workspace_shadow)?;
                }
            }
            _ => unknown_key("overview", &ks)?,
        }
    }
    Ok(())
}

fn parse_workspace_shadow(table: &LuaTable, ws: &mut WorkspaceShadow) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("workspace-shadow error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => ws.off = value_is_truthy(&value),
            "softness" => ws.softness = value_to_f64(&value)?,
            "spread" => ws.spread = value_to_f64(&value)?,
            "color" => ws.color = parse_css_color(&value_to_string(&value)?)?,
            "offset" => {
                if let Value::Table(t) = &value {
                    ws.offset.x = FloatOrInt(t.get("x").unwrap_or(0.0f64));
                    ws.offset.y = FloatOrInt(t.get("y").unwrap_or(0.0f64));
                }
            }
            _ => unknown_key("workspaces.shadow", &ks)?,
        }
    }
    Ok(())
}

fn parse_xwayland_satellite_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("xwayland-satellite error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => config.xwayland_satellite.off = value_is_truthy(&value),
            "path" => config.xwayland_satellite.path = value_to_string(&value)?,
            _ => unknown_key("xwayland_satellite", &ks)?,
        }
    }
    Ok(())
}

fn parse_debug_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let debug = &mut config.debug;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("debug error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "render_drm_device" | "render-drm-device" => {
                debug.render_drm_device = Some(value_to_string(&value)?.into());
            }
            "ignore_drm_device" | "ignore-drm-device" => {
                if let Value::Table(t) = &value {
                    let mut devices = Vec::new();
                    for v in t.clone().sequence_values::<String>() {
                        devices.push(v.map_err(|e| format!("{e}"))?.into());
                    }
                    debug.ignored_drm_devices = devices;
                } else {
                    debug
                        .ignored_drm_devices
                        .push(value_to_string(&value)?.into());
                }
            }
            "enable_overlay_planes" | "enable-overlay-planes" => {
                if let Value::Boolean(b) = &value {
                    debug.enable_overlay_planes = *b;
                }
            }
            "disable_cursor_plane" | "disable-cursor-plane" => {
                if let Value::Boolean(b) = &value {
                    debug.disable_cursor_plane = *b;
                }
            }
            "disable_direct_scanout" | "disable-direct-scanout" => {
                if let Value::Boolean(b) = &value {
                    debug.disable_direct_scanout = *b;
                }
            }
            "disable_10bit_output" | "disable-10bit-output" => {
                debug.disable_10bit_output = value_is_truthy(&value);
            }
            "disable_pipewire_dmabuf" | "disable-pipewire-dmabuf" => {
                debug.disable_pipewire_dmabuf = value_is_truthy(&value);
            }
            "nested_show_host_cursor" | "nested-show-host-cursor" => {
                debug.nested_show_host_cursor = value_is_truthy(&value);
            }
            _ => unknown_key("debug", &ks)?,
        }
    }
    Ok(())
}

fn parse_switch_events_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let se = &mut config.switch_events;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("switch-events error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "lid_open" | "lid-open" => {
                if let Value::Table(t) = &value {
                    let spawn: Vec<String> = t
                        .get("spawn")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or_default();
                    se.lid_open = Some(SwitchAction { spawn });
                }
            }
            "lid_close" | "lid-close" => {
                if let Value::Table(t) = &value {
                    let spawn: Vec<String> = t
                        .get("spawn")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or_default();
                    se.lid_close = Some(SwitchAction { spawn });
                }
            }
            "tablet_mode_on" | "tablet-mode-on" => {
                if let Value::Table(t) = &value {
                    let spawn: Vec<String> = t
                        .get("spawn")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or_default();
                    se.tablet_mode_on = Some(SwitchAction { spawn });
                }
            }
            "tablet_mode_off" | "tablet-mode-off" => {
                if let Value::Table(t) = &value {
                    let spawn: Vec<String> = t
                        .get("spawn")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or_default();
                    se.tablet_mode_off = Some(SwitchAction { spawn });
                }
            }
            _ => unknown_key("switch_events", &ks)?,
        }
    }
    Ok(())
}

fn parse_environment_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let mut vars = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("environment error: {e}"))?;
        let name = key_str(&key)?;
        let val = match &value {
            Value::String(s) => Some(s.to_str().map_err(|e| format!("{e}"))?.to_string()),
            Value::Nil => None,
            _ => Some(value_to_string(&value)?),
        };
        vars.push(EnvironmentVariable { name, value: val });
    }
    config.environment.0 = vars;
    Ok(())
}

/// Checks that a recent windows switcher bind can work, and doesn't conflict with an earlier one.
fn validate_recent_windows_bind(name: &str, bind: &Bind, previous: &[Bind]) -> Result<(), String> {
    // A modifier is required because the switcher remains on screen as long as any modifier is
    // held. The trigger's own modifier doesn't count, since releasing the trigger releases it.
    let mut modifiers = bind.key.modifiers;
    modifiers &= !bind.key.trigger.implied_modifier();
    if modifiers.is_empty() {
        return Err(format!(
            "recent_windows bind `{name}` must have a modifier key other than the trigger"
        ));
    }

    // FIXME: To support this, all the mods_with_mouse_binds()/mods_with_*wheel_binds()/etc.
    // will need to learn about recent-windows bindings.
    if !bind.key.trigger.is_keyboard() {
        return Err(format!(
            "recent_windows bind `{name}` must use a keyboard key (others are unsupported here for now)"
        ));
    }

    if previous.iter().any(|prev| prev.key.conflicts_with(&bind.key)) {
        return Err(format!(
            "recent_windows bind `{name}` conflicts with another recent_windows bind"
        ));
    }

    Ok(())
}

fn parse_recent_windows_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let rw = &mut config.recent_windows;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("recent-windows error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => {
                rw.on = false;
            }
            "debounce_ms" | "debounce-ms" => {
                if let Ok(v) = value_to_u64(&value) {
                    rw.debounce_ms = v as u16;
                }
            }
            "open_delay_ms" | "open-delay-ms" => {
                if let Ok(v) = value_to_u64(&value) {
                    rw.open_delay_ms = v as u16;
                }
            }
            "highlight" => {
                if let Value::Table(t) = &value {
                    if let Ok(s) = t
                        .get::<String>("active_color")
                        .or_else(|_| t.get("active-color"))
                    {
                        if let Ok(c) = parse_css_color(&s) {
                            rw.highlight.active_color = c;
                        }
                    }
                    rw.highlight.padding = t.get("padding").unwrap_or(rw.highlight.padding);
                }
            }
            "previews" => {
                if let Value::Table(t) = &value {
                    rw.previews.max_height = t.get("max_height").unwrap_or(rw.previews.max_height);
                }
            }
            "binds" => {
                if let Value::Table(t) = &value {
                    let mut binds: Vec<Bind> = Vec::new();
                    for bpair in t.clone().pairs::<Value, Value>() {
                        let (bk, bv) = bpair.map_err(|e| format!("{e}"))?;
                        let bks = key_str(&bk)?;
                        let Ok(bind) = parse_bind_entry(&bks, bv) else {
                            continue;
                        };
                        if let Err(err) = validate_recent_windows_bind(&bks, &bind, &binds) {
                            invalid_config(&err)?;
                            continue;
                        }
                        binds.push(bind);
                    }
                    rw.binds = binds;
                }
            }
            _ => unknown_key("recent_windows", &ks)?,
        }
    }
    Ok(())
}

fn parse_binds_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let mut bvec = Vec::new();
    let mut seen_keys: Vec<(String, Key)> = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("binds error: {e}"))?;
        // Sequence entries produced by neori.bind carry their key inside the entry;
        // map-style entries (from `return { binds = { ... } }`) use the table key.
        let (key_name, action) = match key {
            Value::Integer(_) => {
                let entry = match value {
                    Value::Table(t) => t,
                    _ => {
                        return Err(format!(
                            "binds error: expected a table with key and action fields, got {:?}",
                            value.type_name()
                        ))
                    }
                };
                let entry_key: String =
                    entry.get("key").map_err(|e| format!("binds error: {e}"))?;
                let entry_action: Value = entry
                    .get("action")
                    .map_err(|e| format!("binds error: {e}"))?;
                (entry_key, entry_action)
            }
            k => (key_str(&k)?, value),
        };
        let bind = parse_bind_entry(&key_name, action)?;
        if let Some((prev_name, prev_key)) = seen_keys
            .iter()
            .find(|(_, prev_key)| prev_key.conflicts_with(&bind.key))
        {
            return Err(if *prev_name == key_name || *prev_key == bind.key {
                format!("duplicate keybind: {key_name}")
            } else {
                format!(
                    "conflicting keybinds: `{prev_name}` and `{key_name}` are triggered by the same key press"
                )
            });
        }
        seen_keys.push((key_name, bind.key));
        bvec.push(bind);
    }
    config.binds.0 = bvec;
    Ok(())
}

fn parse_bind_entry(key_name: &str, value: Value) -> Result<Bind, String> {
    let mut key: Key = key_name
        .parse::<Key>()
        .map_err(|e| format!("invalid keybind '{key_name}': {e}"))?;

    // An action given directly, or in `press` / `release` slots.
    let mut action: Option<Action> = None;
    let mut press_action: Option<Action> = None;
    let mut release_action: Option<Action> = None;
    // `None` means the default: repeat unless the bind has a release action.
    let mut repeat: Option<bool> = None;
    let mut cooldown = None;
    let mut allow_when_locked = false;
    let mut allow_inhibiting = true;
    let mut pass_through: Option<bool> = None;
    // `None` means the default (keycode when possible, falling back to keysym for keys
    // without a US-layout keycode such as XF86 media keys).
    let mut by_keycode: Option<bool> = None;
    let mut hotkey_overlay_title: Option<String> = None;
    let mut has_hotkey_overlay_title = false;

    match &value {
        Value::String(s) => {
            action = Some(parse_action_string(&s.to_str().map_err(|e| format!("{e}"))?)?);
        }
        Value::Table(t) => {
            for pair in t.clone().pairs::<Value, Value>() {
                let (ak, av) = pair.map_err(|e| format!("bind action error: {e}"))?;
                let aks = key_str(&ak)?;
                if aks == "script_fn_id" {
                    let id = value_to_u64(&av).map_err(|e| format!("script_fn_id: {e}"))?;
                    action = Some(Action::ScriptFunction { id });
                    continue;
                }
                if aks == "press" || aks == "release" {
                    let slot = if aks == "press" {
                        &mut press_action
                    } else {
                        &mut release_action
                    };
                    *slot = Some(
                        parse_bind_slot_action(&av)
                            .map_err(|e| format!("bind {key_name}: {aks}: {e}"))?,
                    );
                    continue;
                }
                if let Some(a) = action_from_table_key(t, &aks, &av)? {
                    action = Some(a);
                    continue;
                }
                match aks.as_str() {
                    "repeat" => {
                        repeat = Some(value_is_truthy(&av));
                    }
                    "cooldown_ms" => {
                        if let Ok(ms) = value_to_u64(&av) {
                            cooldown = Some(Duration::from_millis(ms));
                        }
                    }
                    "allow_when_locked" | "allow-when-locked" => {
                        allow_when_locked = value_is_truthy(&av);
                    }
                    "allow_inhibiting" | "allow-inhibiting" => {
                        allow_inhibiting = value_is_truthy(&av);
                    }
                    "pass_through" | "pass-through" => {
                        pass_through = Some(value_is_truthy(&av));
                    }
                    "keycode" | "key-code" => {
                        by_keycode = Some(value_is_truthy(&av));
                    }
                    "hotkey_overlay_title" | "hotkey-overlay-title" => {
                        has_hotkey_overlay_title = true;
                        if let Value::String(s) = &av {
                            hotkey_overlay_title =
                                Some(s.to_str().map_err(|e| format!("{e}"))?.to_string());
                        } else {
                            hotkey_overlay_title = None;
                        }
                    }
                    "skip_confirmation" | "skip-confirmation" | "write_to_disk"
                    | "write-to-disk" | "show_pointer" | "show-pointer" | "delay_ms"
                    | "delay-ms" | "path" => {}
                    _ => unknown_key("bind entry", &aks)?,
                }
            }
        }
        _ => {}
    }

    if action.is_some() && (press_action.is_some() || release_action.is_some()) {
        return Err(format!(
            "bind {key_name}: cannot mix a direct action with press/release actions"
        ));
    }
    let press_action = press_action.or(action);

    if release_action.is_some()
        && matches!(
            key.trigger,
            Trigger::WheelScrollDown
                | Trigger::WheelScrollUp
                | Trigger::WheelScrollLeft
                | Trigger::WheelScrollRight
                | Trigger::TouchpadScrollDown
                | Trigger::TouchpadScrollUp
                | Trigger::TouchpadScrollLeft
                | Trigger::TouchpadScrollRight
        )
    {
        return Err(format!(
            "bind {key_name}: release actions are not supported for scroll binds"
        ));
    }

    if repeat == Some(true) && press_action.is_none() && release_action.is_some() {
        return Err(format!("bind {key_name}: release-only binds cannot repeat"));
    }
    let repeat = repeat.unwrap_or(release_action.is_none());

    // Supporting this for mouse binds would need design: they trigger compositor gestures, and
    // it's unclear whether a pass-through should also trigger the gesture. Keyboard only for now.
    if pass_through.is_some() && !key.trigger.is_keyboard() {
        return Err(format!(
            "bind {key_name}: pass_through is only supported for keyboard binds"
        ));
    }

    // The toggle-inhibit action must always be uninhibitable.
    // Otherwise, it would be impossible to trigger it.
    if matches!(press_action, Some(Action::ToggleKeyboardShortcutsInhibit {}))
        || matches!(release_action, Some(Action::ToggleKeyboardShortcutsInhibit {}))
    {
        allow_inhibiting = false;
    }

    let action = match (press_action, release_action) {
        (Some(press), Some(release)) => BoundAction::Both { press, release },
        (Some(press), None) => BoundAction::Press(press),
        (None, Some(release)) => BoundAction::Release(release),
        (None, None) => {
            return Err(format!(
                "bind {key_name}: no action or Lua function provided"
            ));
        }
    };

    match by_keycode {
        // Explicitly disabled: keep the layout-dependent keysym trigger.
        Some(false) => {}
        // Explicitly enabled: convert, erroring if the key has no US-layout keycode.
        Some(true) => {
            key = key
                .into_keycode()
                .map_err(|e| format!("invalid keybind '{key_name}': {e}"))?;
        }
        // Default: convert when possible, otherwise silently keep the keysym (e.g. XF86 keys).
        None => {
            if let Some(converted) = key.try_into_keycode() {
                key = converted;
            }
        }
    }

    Ok(Bind {
        key,
        action,
        repeat,
        cooldown,
        allow_when_locked,
        allow_inhibiting,
        pass_through,
        hotkey_overlay_title: if has_hotkey_overlay_title {
            Some(hotkey_overlay_title)
        } else {
            None
        },
    })
}

/// Parses the action in a bind's `press` or `release` slot: anything a bind itself takes (an
/// action string, an action table like `neori.spawn(...)` returns, or a Lua function).
fn parse_bind_slot_action(value: &Value) -> Result<Action, String> {
    match value {
        Value::String(s) => parse_action_string(&s.to_str().map_err(|e| format!("{e}"))?),
        Value::Table(t) => {
            let mut action = None;
            for pair in t.clone().pairs::<Value, Value>() {
                let (ak, av) = pair.map_err(|e| format!("bind action error: {e}"))?;
                let aks = key_str(&ak)?;
                let parsed = if aks == "script_fn_id" {
                    let id = value_to_u64(&av).map_err(|e| format!("script_fn_id: {e}"))?;
                    Some(Action::ScriptFunction { id })
                } else {
                    action_from_table_key(t, &aks, &av)?
                };
                match parsed {
                    Some(parsed) => {
                        if action.replace(parsed).is_some() {
                            return Err("only one action is allowed".to_string());
                        }
                    }
                    None => match aks.as_str() {
                        // Arguments of the action itself.
                        "skip_confirmation" | "skip-confirmation" | "write_to_disk"
                        | "write-to-disk" | "show_pointer" | "show-pointer" | "delay_ms"
                        | "delay-ms" | "path" => {}
                        _ => {
                            return Err(format!(
                                "unexpected key `{aks}` (bind options go on the bind itself)"
                            ))
                        }
                    },
                }
            }
            action.ok_or_else(|| "expected an action".to_string())
        }
        other => Err(format!(
            "expected an action string, table or function, got {}",
            other.type_name()
        )),
    }
}

fn parse_spawn_args(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(s) => Ok(vec![s.to_str().map_err(|e| format!("{e}"))?.to_string()]),
        Value::Table(t) => {
            let mut args = Vec::new();
            for v in t.clone().sequence_values::<Value>() {
                let v = v.map_err(|e| format!("spawn arg error: {e}"))?;
                args.push(value_to_string(&v)?);
            }
            Ok(args)
        }
        _ => Ok(vec![]),
    }
}

/// Builds a spawn action table for `neori.spawn` (argv) and `neori.spawn_sh` (shell).
///
/// Arguments are flattened into a command: strings are a single argv entry and tables are
/// spliced in as argv lists. A trailing table with string keys is treated as bind metadata
/// (e.g. `hotkey_overlay_title`, `repeat`, `cooldown_ms`) and merged into the action.
fn spawn_action(lua: &Lua, args: Vec<Value>, shell: bool) -> Result<LuaTable, String> {
    let mut args = args;
    let mut opts = None;
    if let Some(Value::Table(t)) = args.last() {
        if table_is_map(t)? {
            opts = Some(t.clone());
            args.pop();
        }
    }

    if args.is_empty() {
        return Err("spawn requires a command".to_string());
    }

    let action = lua.create_table().map_err(|e| e.to_string())?;
    if shell {
        let mut command = String::new();
        for (i, value) in args.iter().enumerate() {
            if i > 0 {
                command.push(' ');
            }
            command.push_str(&value_to_string(value)?);
        }
        action.set("spawn_sh", command).map_err(|e| e.to_string())?;
    } else {
        let mut argv = Vec::new();
        for value in &args {
            match value {
                Value::Table(t) => {
                    for entry in t.clone().sequence_values::<Value>() {
                        let entry = entry.map_err(|e| e.to_string())?;
                        argv.push(value_to_string(&entry)?);
                    }
                }
                _ => argv.push(value_to_string(value)?),
            }
        }
        let argv_table = lua.create_table().map_err(|e| e.to_string())?;
        for (i, arg) in argv.iter().enumerate() {
            argv_table
                .set(i + 1, arg.clone())
                .map_err(|e| e.to_string())?;
        }
        action.set("spawn", argv_table).map_err(|e| e.to_string())?;
    }

    if let Some(opts) = opts {
        for pair in opts.pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|e| e.to_string())?;
            action.set(key, value).map_err(|e| e.to_string())?;
        }
    }

    Ok(action)
}

/// Keys inside a `create_virtual_output` action table that are action-shaped (width,
/// height, refresh_rate, name) vs. bind metadata (hotkey_overlay_title, cooldown_ms, …).
const CREATE_VIRTUAL_OUTPUT_ACTION_KEYS: &[&str] =
    &["name", "width", "height", "refresh_rate", "refresh-rate"];

/// Builds a `create_virtual_output` action table from Lua args.
///
/// Accepts either a single table of options or no arguments at all (defaults). Non-action keys
/// in the table (string-keyed, not listed in [`CREATE_VIRTUAL_OUTPUT_ACTION_KEYS`]) are merged
/// in as bind metadata, mirroring `spawn_action`.
fn create_virtual_output_action(lua: &Lua, args: Vec<Value>) -> Result<LuaTable, String> {
    let opts = match args.as_slice() {
        [] => None,
        [Value::Table(t)] => Some(t.clone()),
        _ => {
            return Err(
                "create_virtual_output expects a single options table or no arguments"
                    .to_string(),
            )
        }
    };

    let inner = lua.create_table().map_err(|e| e.to_string())?;
    let action = lua.create_table().map_err(|e| e.to_string())?;

    if let Some(ref opts) = opts {
        for pair in opts.clone().pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|e| e.to_string())?;
            let key_str = match &key {
                Value::String(s) => Some(s.to_str().map_err(|e| format!("{e}"))?.to_string()),
                _ => None,
            };
            if let Some(key_str) = key_str {
                if CREATE_VIRTUAL_OUTPUT_ACTION_KEYS.contains(&key_str.as_str()) {
                    // Normalize the kebab-case alias to snake_case.
                    let normalized = if key_str == "refresh-rate" {
                        "refresh_rate"
                    } else {
                        key_str.as_str()
                    };
                    inner.set(normalized, value).map_err(|e| e.to_string())?;
                } else {
                    action.set(key, value).map_err(|e| e.to_string())?;
                }
            } else {
                action.set(key, value).map_err(|e| e.to_string())?;
            }
        }
    }

    action
        .set("create_virtual_output", inner)
        .map_err(|e| e.to_string())?;
    Ok(action)
}

/// Builds a `remove_virtual_output` action table from Lua args.
///
/// Accepts either a bare string (`neori.remove_virtual_output("sunshine")`) or a table with a
/// `name` field and optional metadata.
fn remove_virtual_output_action(lua: &Lua, args: Vec<Value>) -> Result<LuaTable, String> {
    let (name, opts) = match args.as_slice() {
        [Value::String(s)] => (s.to_str().map_err(|e| format!("{e}"))?.to_string(), None),
        [Value::Table(t)] => {
            let name = t
                .get::<Option<String>>("name")
                .map_err(|e| format!("{e}"))?
                .ok_or_else(|| {
                    "remove_virtual_output requires a `name` field in the options table"
                        .to_string()
                })?;
            (name, Some(t.clone()))
        }
        _ => {
            return Err(
                "remove_virtual_output expects a string name or an options table with `name`"
                    .to_string(),
            )
        }
    };

    let inner = lua.create_table().map_err(|e| e.to_string())?;
    inner.set("name", name).map_err(|e| e.to_string())?;

    let action = lua.create_table().map_err(|e| e.to_string())?;
    if let Some(opts) = opts {
        for pair in opts.pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|e| e.to_string())?;
            if let Value::String(ref s) = key {
                if s.to_str().map_err(|e| format!("{e}"))? == "name" {
                    continue;
                }
            }
            action.set(key, value).map_err(|e| e.to_string())?;
        }
    }
    action
        .set("remove_virtual_output", inner)
        .map_err(|e| e.to_string())?;
    Ok(action)
}

/// Returns whether `t` has at least one string key, i.e. it is used as a map (bind
/// metadata) rather than a plain argv sequence.
fn table_is_map(t: &LuaTable) -> Result<bool, String> {
    for pair in t.clone().pairs::<Value, Value>() {
        let (key, _) = pair.map_err(|e| e.to_string())?;
        if matches!(key, Value::String(_)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_size_change_str(s: &str) -> Result<SizeChange, String> {
    s.parse::<SizeChange>()
        .map_err(|e| format!("invalid size change '{s}': {e}"))
}

fn parse_size_change(value: &Value) -> Result<SizeChange, String> {
    parse_size_change_str(&value_to_string(value)?)
}

/// Resolves `name` + optional Lua `arg` into a dispatchable [`Action`] at runtime.
///
/// Used by `neori.action(name, args)`. Reuses the config-time [`parse_action_string`] and
/// [`action_from_table_key`] pipeline so the runtime surface matches the declarative one.
///
/// `wrapper` is a freshly-built Lua table that holds `{ [name] = effective_arg }`; the
/// caller provides it because it already has the live Lua context. The caller must set the
/// `name` key on `wrapper` before calling.
fn resolve_runtime_action(
    name: &str,
    arg: &Value,
    wrapper: &LuaTable,
) -> Result<Action, String> {
    // Nil arg: try the no-argument string form first (handles "focus-column-left",
    // "close-window", "toggle-overview", "create-virtual-output", …).
    if matches!(arg, Value::Nil) {
        if let Ok(action) = parse_action_string(name) {
            return Ok(action);
        }
    }

    if let Some(action) = action_from_table_key(wrapper, name, arg)? {
        return Ok(action);
    }

    // Final fallback: the string form (may still fail with "unknown action").
    parse_action_string(name)
}

fn parse_action_string(s: &str) -> Result<Action, String> {
    let action = match s {
        "close_window" | "close-window" => Action::CloseWindow { id: None },
        "fullscreen_window" | "fullscreen-window" => Action::FullscreenWindow { id: None },
        "quit" => Action::Quit {
            skip_confirmation: false,
        },
        "toggle_keyboard_shortcuts_inhibit" | "toggle-keyboard-shortcuts-inhibit" => {
            Action::ToggleKeyboardShortcutsInhibit {}
        }
        "toggle_window_floating" | "toggle-window-floating" => {
            Action::ToggleWindowFloating { id: None }
        }
        "consume_window_into_column" | "consume-window-into-column" => {
            Action::ConsumeWindowIntoColumn {}
        }
        "expel_window_from_column" | "expel-window-from-column" => Action::ExpelWindowFromColumn {},
        "focus_column_left" | "focus-column-left" => Action::FocusColumnLeft {},
        "focus_column_right" | "focus-column-right" => Action::FocusColumnRight {},
        "focus_window_down" | "focus-window-down" => Action::FocusWindowDown {},
        "focus_window_up" | "focus-window-up" => Action::FocusWindowUp {},
        "focus_monitor_left" | "focus-monitor-left" => Action::FocusMonitorLeft {},
        "focus_monitor_right" | "focus-monitor-right" => Action::FocusMonitorRight {},
        "focus_monitor_down" | "focus-monitor-down" => Action::FocusMonitorDown {},
        "focus_monitor_up" | "focus-monitor-up" => Action::FocusMonitorUp {},
        "focus_workspace_down" | "focus-workspace-down" => Action::FocusWorkspaceDown {},
        "focus_workspace_up" | "focus-workspace-up" => Action::FocusWorkspaceUp {},
        "focus_workspace_previous" | "focus-workspace-previous" => {
            Action::FocusWorkspacePrevious {}
        }
        "move_column_left" | "move-column-left" => Action::MoveColumnLeft {},
        "move_column_right" | "move-column-right" => Action::MoveColumnRight {},
        "move_window_down" | "move-window-down" => Action::MoveWindowDown {},
        "move_window_up" | "move-window-up" => Action::MoveWindowUp {},
        "switch_preset_column_width" | "switch-preset-column-width" => {
            Action::SwitchPresetColumnWidth {}
        }
        "switch_preset_column_width_back" | "switch-preset-column-width-back" => {
            Action::SwitchPresetColumnWidthBack {}
        }
        "maximize_column" | "maximize-column" => Action::MaximizeColumn {},
        "maximize_window_to_edges" | "maximize-window-to-edges" => {
            Action::MaximizeWindowToEdges { id: None }
        }
        "center_column" | "center-column" => Action::CenterColumn {},
        "toggle_overview" | "toggle-overview" => Action::ToggleOverview {},
        "open_overview" | "open-overview" => Action::OpenOverview {},
        "close_overview" | "close-overview" => Action::CloseOverview {},
        "enlarge_cursor" | "enlarge-cursor" => Action::EnlargeCursor {},
        "send_to_scratchpad" | "send-to-scratchpad" => Action::SendToScratchpad { name: None },
        "remove_from_scratchpad" | "remove-from-scratchpad" => {
            Action::RemoveFromScratchpad { name: None }
        }
        "toggle_scratchpad" | "toggle-scratchpad" => Action::ToggleScratchpad { name: None },
        _ if let Some(name) = s
            .strip_prefix("send-to-scratchpad ")
            .or_else(|| s.strip_prefix("send_to_scratchpad ")) =>
        {
            Action::SendToScratchpad {
                name: Some(name.trim().to_string()),
            }
        }
        _ if let Some(name) = s
            .strip_prefix("remove-from-scratchpad ")
            .or_else(|| s.strip_prefix("remove_from_scratchpad ")) =>
        {
            Action::RemoveFromScratchpad {
                name: Some(name.trim().to_string()),
            }
        }
        _ if let Some(name) = s
            .strip_prefix("toggle-scratchpad ")
            .or_else(|| s.strip_prefix("toggle_scratchpad ")) =>
        {
            Action::ToggleScratchpad {
                name: Some(name.trim().to_string()),
            }
        }
        "show_hotkey_overlay" | "show-hotkey-overlay" => Action::ShowHotkeyOverlay {},
        "screenshot" => Action::Screenshot {
            show_pointer: true,
            path: None,
        },
        "screenshot_screen" | "screenshot-screen" => Action::ScreenshotScreen {
            write_to_disk: true,
            show_pointer: true,
            path: None,
        },
        "screenshot_window" | "screenshot-window" => Action::ScreenshotWindow {
            id: None,
            write_to_disk: true,
            show_pointer: false,
            path: None,
        },
        "power_off_monitors" | "power-off-monitors" => Action::PowerOffMonitors {},
        "toggle_windowed_fullscreen" | "toggle-windowed-fullscreen" => {
            Action::ToggleWindowedFullscreen { id: None }
        }
        "focus_window_previous" | "focus-window-previous" => Action::FocusWindowPrevious {},
        "move_window_to_workspace_down" | "move-window-to-workspace-down" => {
            Action::MoveWindowToWorkspaceDown { focus: true }
        }
        "move_window_to_workspace_up" | "move-window-to-workspace-up" => {
            Action::MoveWindowToWorkspaceUp { focus: true }
        }
        "move_column_to_workspace_down" | "move-column-to-workspace-down" => {
            Action::MoveColumnToWorkspaceDown { focus: true }
        }
        "move_column_to_workspace_up" | "move-column-to-workspace-up" => {
            Action::MoveColumnToWorkspaceUp { focus: true }
        }
        "toggle_column_tabbed_display" | "toggle-column-tabbed-display" => {
            Action::ToggleColumnTabbedDisplay {}
        }
        "focus_column_first" | "focus-column-first" => Action::FocusColumnFirst {},
        "focus_column_last" | "focus-column-last" => Action::FocusColumnLast {},
        "move_column_to_first" | "move-column-to-first" => Action::MoveColumnToFirst {},
        "move_column_to_last" | "move-column-to-last" => Action::MoveColumnToLast {},
        "move_column_to_monitor_left" | "move-column-to-monitor-left" => {
            Action::MoveColumnToMonitorLeft {}
        }
        "move_column_to_monitor_right" | "move-column-to-monitor-right" => {
            Action::MoveColumnToMonitorRight {}
        }
        "move_column_to_monitor_down" | "move-column-to-monitor-down" => {
            Action::MoveColumnToMonitorDown {}
        }
        "move_column_to_monitor_up" | "move-column-to-monitor-up" => {
            Action::MoveColumnToMonitorUp {}
        }
        "move_workspace_to_monitor_left" | "move-workspace-to-monitor-left" => {
            Action::MoveWorkspaceToMonitorLeft {}
        }
        "move_workspace_to_monitor_right" | "move-workspace-to-monitor-right" => {
            Action::MoveWorkspaceToMonitorRight {}
        }
        "move_workspace_to_monitor_down" | "move-workspace-to-monitor-down" => {
            Action::MoveWorkspaceToMonitorDown {}
        }
        "move_workspace_to_monitor_up" | "move-workspace-to-monitor-up" => {
            Action::MoveWorkspaceToMonitorUp {}
        }
        "move_workspace_down" | "move-workspace-down" => Action::MoveWorkspaceDown {},
        "move_workspace_up" | "move-workspace-up" => Action::MoveWorkspaceUp {},
        "consume_or_expel_window_left" | "consume-or-expel-window-left" => {
            Action::ConsumeOrExpelWindowLeft { id: None }
        }
        "consume_or_expel_window_right" | "consume-or-expel-window-right" => {
            Action::ConsumeOrExpelWindowRight { id: None }
        }
        "switch_preset_window_height" | "switch-preset-window-height" => {
            Action::SwitchPresetWindowHeight { id: None }
        }
        "switch_preset_window_height_back" | "switch-preset-window-height-back" => {
            Action::SwitchPresetWindowHeightBack { id: None }
        }
        "reset_window_height" | "reset-window-height" => Action::ResetWindowHeight { id: None },
        "expand_column_to_available_width" | "expand-column-to-available-width" => {
            Action::ExpandColumnToAvailableWidth {}
        }
        "center_visible_columns" | "center-visible-columns" => Action::CenterVisibleColumns {},
        "switch_focus_between_floating_and_tiling" | "switch-focus-between-floating-and-tiling" => {
            Action::SwitchFocusBetweenFloatingAndTiling {}
        }
        "do_screen_transition" | "do-screen-transition" => {
            Action::DoScreenTransition { delay_ms: None }
        }
        "toggle_window_rule_opacity" | "toggle-window-rule-opacity" => {
            Action::ToggleWindowRuleOpacity { id: None }
        }
        "toggle_debug_tint" | "toggle-debug-tint" => Action::ToggleDebugTint {},
        "toggle_touchpad" | "toggle-touchpad" => Action::ToggleTouchpad {},
        "debug_toggle_opaque_regions" | "debug-toggle-opaque-regions" => {
            Action::DebugToggleOpaqueRegions {}
        }
        "debug_toggle_damage" | "debug-toggle-damage" => Action::DebugToggleDamage {},
        _ if let Some(v) = s
            .strip_prefix("set-column-width ")
            .or_else(|| s.strip_prefix("set_column_width ")) =>
        {
            Action::SetColumnWidth {
                change: parse_size_change_str(v)?,
            }
        }
        _ if let Some(v) = s
            .strip_prefix("set-window-height ")
            .or_else(|| s.strip_prefix("set_window_height ")) =>
        {
            Action::SetWindowHeight {
                id: None,
                change: parse_size_change_str(v)?,
            }
        }
        _ if let Some(v) = s
            .strip_prefix("set-window-width ")
            .or_else(|| s.strip_prefix("set_window_width ")) =>
        {
            Action::SetWindowWidth {
                id: None,
                change: parse_size_change_str(v)?,
            }
        }
        _ if let Some(cmd) = s.strip_prefix("spawn ") => Action::Spawn {
            command: vec![cmd.to_string()],
        },
        _ if let Some(cmd) = s
            .strip_prefix("spawn-sh ")
            .or_else(|| s.strip_prefix("spawn_sh ")) =>
        {
            Action::SpawnSh {
                command: cmd.to_string(),
            }
        }
        "create_virtual_output" | "create-virtual-output" => Action::CreateVirtualOutput {
            name: None,
            width: None,
            height: None,
            refresh_rate: None,
        },
        _ if let Some(name) = s
            .strip_prefix("remove-virtual-output ")
            .or_else(|| s.strip_prefix("remove_virtual_output ")) =>
        {
            Action::RemoveVirtualOutput {
                name: name.to_string(),
            }
        }
        _ => return Err(format!("unknown action: {s}")),
    };
    Ok(action)
}

/// Parses the value of a scratchpad action table key into an optional scratchpad name.
///
/// Accepts `true` (default scratchpad), a string name, or a table with a `name` field.
fn scratchpad_name_from_value(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Nil | Value::Boolean(_) => Ok(None),
        Value::String(s) => Ok(Some(s.to_str().map_err(|e| format!("{e}"))?.to_string())),
        Value::Table(t) => t.get::<Option<String>>("name").map_err(|e| format!("{e}")),
        _ => Err(
            "scratchpad action expects `true`, a name string, or a table with `name`".to_string(),
        ),
    }
}

/// Maps a single action-table key (e.g. `spawn`) to its `Action`. Returns `None` for
/// keys that are not actions (bind metadata such as `repeat` or `cooldown_ms`).
fn action_from_table_key(
    table: &LuaTable,
    key: &str,
    value: &Value,
) -> Result<Option<Action>, String> {
    let action = match key {
        "spawn" => Some(Action::Spawn {
            command: parse_spawn_args(value)?,
        }),
        "spawn_sh" | "spawn-sh" => Some(Action::SpawnSh {
            command: value_to_string(value)?,
        }),
        "create_virtual_output" | "create-virtual-output" => {
            let (name, width, height, refresh_rate) = match value {
                Value::Table(t) => (
                    t.get::<Option<String>>("name").map_err(|e| format!("{e}"))?,
                    t.get::<Option<u16>>("width").map_err(|e| format!("{e}"))?,
                    t.get::<Option<u16>>("height").map_err(|e| format!("{e}"))?,
                    {
                        let v: Value =
                            table_get_alias(t, "refresh_rate", "refresh-rate")?;
                        match v {
                            Value::Nil => None,
                            Value::Integer(i) => Some(i as u32),
                            Value::Number(n) => Some(n.round() as u32),
                            other => {
                                return Err(format!(
                                    "create_virtual_output.refresh_rate: expected number, got {:?}",
                                    other
                                ))
                            }
                        }
                    },
                ),
                // Bare `create_virtual_output = true` is shorthand for "all defaults".
                Value::Nil | Value::Boolean(_) => (None, None, None, None),
                _ => {
                    return Err(
                        "create_virtual_output expects a table of options".to_string()
                    )
                }
            };
            Some(Action::CreateVirtualOutput {
                name,
                width,
                height,
                refresh_rate,
            })
        }
        "remove_virtual_output" | "remove-virtual-output" => {
            let name = match value {
                Value::String(s) => s.to_str().map_err(|e| format!("{e}"))?.to_string(),
                Value::Table(t) => t
                    .get::<Option<String>>("name")
                    .map_err(|e| format!("{e}"))?
                    .ok_or_else(|| {
                        "remove_virtual_output requires a `name`".to_string()
                    })?,
                _ => {
                    return Err(
                        "remove_virtual_output expects a string name or a table with `name`"
                            .to_string(),
                    )
                }
            };
            Some(Action::RemoveVirtualOutput { name })
        }
        "close_window" | "close-window" => Some(Action::CloseWindow { id: None }),
        "fullscreen_window" | "fullscreen-window" => Some(Action::FullscreenWindow { id: None }),
        "toggle_window_floating" | "toggle-window-floating" => {
            Some(Action::ToggleWindowFloating { id: None })
        }
        "consume_window_into_column" | "consume-window-into-column" => {
            Some(Action::ConsumeWindowIntoColumn {})
        }
        "expel_window_from_column" | "expel-window-from-column" => {
            Some(Action::ExpelWindowFromColumn {})
        }
        "focus_workspace" | "focus-workspace" => Some(match value {
            Value::Integer(i) => Action::FocusWorkspace {
                reference: WorkspaceReference::Index(*i as u8),
            },
            Value::String(s) => Action::FocusWorkspace {
                reference: WorkspaceReference::Name(
                    s.to_str().map_err(|e| format!("{e}"))?.to_string(),
                ),
            },
            _ => Action::FocusWorkspace {
                reference: WorkspaceReference::Index(0),
            },
        }),
        "move_column_to_workspace" | "move-column-to-workspace" => Some(match value {
            Value::Integer(i) => Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Index(*i as u8),
                focus: true,
            },
            Value::String(s) => Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Name(
                    s.to_str().map_err(|e| format!("{e}"))?.to_string(),
                ),
                focus: true,
            },
            _ => Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Index(0),
                focus: true,
            },
        }),
        "focus_column_left" | "focus-column-left" => Some(Action::FocusColumnLeft {}),
        "focus_column_right" | "focus-column-right" => Some(Action::FocusColumnRight {}),
        "focus_window_down" | "focus-window-down" => Some(Action::FocusWindowDown {}),
        "focus_window_up" | "focus-window-up" => Some(Action::FocusWindowUp {}),
        "focus_monitor_left" | "focus-monitor-left" => Some(Action::FocusMonitorLeft {}),
        "focus_monitor_right" | "focus-monitor-right" => Some(Action::FocusMonitorRight {}),
        "focus_monitor_down" | "focus-monitor-down" => Some(Action::FocusMonitorDown {}),
        "focus_monitor_up" | "focus-monitor-up" => Some(Action::FocusMonitorUp {}),
        "focus_workspace_down" | "focus-workspace-down" => Some(Action::FocusWorkspaceDown {}),
        "focus_workspace_up" | "focus-workspace-up" => Some(Action::FocusWorkspaceUp {}),
        "focus_workspace_previous" | "focus-workspace-previous" => {
            Some(Action::FocusWorkspacePrevious {})
        }
        "move_column_left" | "move-column-left" => Some(Action::MoveColumnLeft {}),
        "move_column_right" | "move-column-right" => Some(Action::MoveColumnRight {}),
        "move_window_down" | "move-window-down" => Some(Action::MoveWindowDown {}),
        "move_window_up" | "move-window-up" => Some(Action::MoveWindowUp {}),
        "switch_preset_column_width" | "switch-preset-column-width" => {
            Some(Action::SwitchPresetColumnWidth {})
        }
        "switch_preset_column_width_back" | "switch-preset-column-width-back" => {
            Some(Action::SwitchPresetColumnWidthBack {})
        }
        "maximize_column" | "maximize-column" => Some(Action::MaximizeColumn {}),
        "maximize_window_to_edges" | "maximize-window-to-edges" => {
            Some(Action::MaximizeWindowToEdges { id: None })
        }
        "center_column" | "center-column" => Some(Action::CenterColumn {}),
        "toggle_overview" | "toggle-overview" => Some(Action::ToggleOverview {}),
        "open_overview" | "open-overview" => Some(Action::OpenOverview {}),
        "close_overview" | "close-overview" => Some(Action::CloseOverview {}),
        "enlarge_cursor" | "enlarge-cursor" => Some(Action::EnlargeCursor {}),
        "send_to_scratchpad" | "send-to-scratchpad" => Some(Action::SendToScratchpad {
            name: scratchpad_name_from_value(value)?,
        }),
        "remove_from_scratchpad" | "remove-from-scratchpad" => Some(Action::RemoveFromScratchpad {
            name: scratchpad_name_from_value(value)?,
        }),
        "toggle_scratchpad" | "toggle-scratchpad" => Some(Action::ToggleScratchpad {
            name: scratchpad_name_from_value(value)?,
        }),
        "show_hotkey_overlay" | "show-hotkey-overlay" => Some(Action::ShowHotkeyOverlay {}),
        "screenshot" => {
            let show_pointer = table
                .get::<Option<bool>>("show_pointer")
                .map_err(|e| format!("{e}"))?
                .unwrap_or(true);
            let path = table
                .get::<Option<String>>("path")
                .map_err(|e| format!("{e}"))?;
            Some(Action::Screenshot { show_pointer, path })
        }
        "screenshot_screen" | "screenshot-screen" => {
            let write_to_disk = table
                .get::<Option<bool>>("write_to_disk")
                .map_err(|e| format!("{e}"))?
                .unwrap_or(true);
            let show_pointer = table
                .get::<Option<bool>>("show_pointer")
                .map_err(|e| format!("{e}"))?
                .unwrap_or(true);
            let path = table
                .get::<Option<String>>("path")
                .map_err(|e| format!("{e}"))?;
            Some(Action::ScreenshotScreen {
                write_to_disk,
                show_pointer,
                path,
            })
        }
        "screenshot_window" | "screenshot-window" => {
            let write_to_disk = table
                .get::<Option<bool>>("write_to_disk")
                .map_err(|e| format!("{e}"))?
                .unwrap_or(true);
            let show_pointer = table
                .get::<Option<bool>>("show_pointer")
                .map_err(|e| format!("{e}"))?
                .unwrap_or(false);
            let path = table
                .get::<Option<String>>("path")
                .map_err(|e| format!("{e}"))?;
            Some(Action::ScreenshotWindow {
                id: None,
                write_to_disk,
                show_pointer,
                path,
            })
        }
        "do_screen_transition" | "do-screen-transition" => {
            let delay_ms = table
                .get::<Option<u16>>("delay_ms")
                .map_err(|e| format!("{e}"))?;
            Some(Action::DoScreenTransition { delay_ms })
        }
        "toggle_window_rule_opacity" | "toggle-window-rule-opacity" => {
            Some(Action::ToggleWindowRuleOpacity { id: None })
        }
        "toggle_debug_tint" | "toggle-debug-tint" => Some(Action::ToggleDebugTint {}),
        "toggle_touchpad" | "toggle-touchpad" => Some(Action::ToggleTouchpad {}),
        "debug_toggle_opaque_regions" | "debug-toggle-opaque-regions" => {
            Some(Action::DebugToggleOpaqueRegions {})
        }
        "debug_toggle_damage" | "debug-toggle-damage" => Some(Action::DebugToggleDamage {}),
        "power_off_monitors" | "power-off-monitors" => Some(Action::PowerOffMonitors {}),
        "toggle_windowed_fullscreen" | "toggle-windowed-fullscreen" => {
            Some(Action::ToggleWindowedFullscreen { id: None })
        }
        "focus_window_previous" | "focus-window-previous" => Some(Action::FocusWindowPrevious {}),
        "move_window_to_workspace_down" | "move-window-to-workspace-down" => {
            Some(Action::MoveWindowToWorkspaceDown { focus: true })
        }
        "move_window_to_workspace_up" | "move-window-to-workspace-up" => {
            Some(Action::MoveWindowToWorkspaceUp { focus: true })
        }
        "move_column_to_workspace_down" | "move-column-to-workspace-down" => {
            Some(Action::MoveColumnToWorkspaceDown { focus: true })
        }
        "move_column_to_workspace_up" | "move-column-to-workspace-up" => {
            Some(Action::MoveColumnToWorkspaceUp { focus: true })
        }
        "toggle_column_tabbed_display" | "toggle-column-tabbed-display" => {
            Some(Action::ToggleColumnTabbedDisplay {})
        }
        "toggle_keyboard_shortcuts_inhibit" | "toggle-keyboard-shortcuts-inhibit" => {
            Some(Action::ToggleKeyboardShortcutsInhibit {})
        }
        "focus_column_first" | "focus-column-first" => Some(Action::FocusColumnFirst {}),
        "focus_column_last" | "focus-column-last" => Some(Action::FocusColumnLast {}),
        "move_column_to_first" | "move-column-to-first" => Some(Action::MoveColumnToFirst {}),
        "move_column_to_last" | "move-column-to-last" => Some(Action::MoveColumnToLast {}),
        "move_column_to_monitor_left" | "move-column-to-monitor-left" => {
            Some(Action::MoveColumnToMonitorLeft {})
        }
        "move_column_to_monitor_right" | "move-column-to-monitor-right" => {
            Some(Action::MoveColumnToMonitorRight {})
        }
        "move_column_to_monitor_down" | "move-column-to-monitor-down" => {
            Some(Action::MoveColumnToMonitorDown {})
        }
        "move_column_to_monitor_up" | "move-column-to-monitor-up" => {
            Some(Action::MoveColumnToMonitorUp {})
        }
        "move_workspace_to_monitor_left" | "move-workspace-to-monitor-left" => {
            Some(Action::MoveWorkspaceToMonitorLeft {})
        }
        "move_workspace_to_monitor_right" | "move-workspace-to-monitor-right" => {
            Some(Action::MoveWorkspaceToMonitorRight {})
        }
        "move_workspace_to_monitor_down" | "move-workspace-to-monitor-down" => {
            Some(Action::MoveWorkspaceToMonitorDown {})
        }
        "move_workspace_to_monitor_up" | "move-workspace-to-monitor-up" => {
            Some(Action::MoveWorkspaceToMonitorUp {})
        }
        "move_workspace_down" | "move-workspace-down" => Some(Action::MoveWorkspaceDown {}),
        "move_workspace_up" | "move-workspace-up" => Some(Action::MoveWorkspaceUp {}),
        "consume_or_expel_window_left" | "consume-or-expel-window-left" => {
            Some(Action::ConsumeOrExpelWindowLeft { id: None })
        }
        "consume_or_expel_window_right" | "consume-or-expel-window-right" => {
            Some(Action::ConsumeOrExpelWindowRight { id: None })
        }
        "switch_preset_window_height" | "switch-preset-window-height" => {
            Some(Action::SwitchPresetWindowHeight { id: None })
        }
        "switch_preset_window_height_back" | "switch-preset-window-height-back" => {
            Some(Action::SwitchPresetWindowHeightBack { id: None })
        }
        "reset_window_height" | "reset-window-height" => {
            Some(Action::ResetWindowHeight { id: None })
        }
        "expand_column_to_available_width" | "expand-column-to-available-width" => {
            Some(Action::ExpandColumnToAvailableWidth {})
        }
        "center_visible_columns" | "center-visible-columns" => {
            Some(Action::CenterVisibleColumns {})
        }
        "switch_focus_between_floating_and_tiling" | "switch-focus-between-floating-and-tiling" => {
            Some(Action::SwitchFocusBetweenFloatingAndTiling {})
        }
        "set_column_width" | "set-column-width" => Some(Action::SetColumnWidth {
            change: parse_size_change(value)?,
        }),
        "set_window_height" | "set-window-height" => Some(Action::SetWindowHeight {
            id: None,
            change: parse_size_change(value)?,
        }),
        "set_window_width" | "set-window-width" => Some(Action::SetWindowWidth {
            id: None,
            change: parse_size_change(value)?,
        }),
        "quit" => {
            let skip = table
                .get::<bool>("skip_confirmation")
                .map_err(|e| format!("{e}"))
                .unwrap_or(false);
            Some(Action::Quit {
                skip_confirmation: skip,
            })
        }
        "repeat"
        | "cooldown_ms"
        | "keycode"
        | "key-code"
        | "allow_when_locked"
        | "allow-when-locked"
        | "allow_inhibiting"
        | "allow-inhibiting"
        | "hotkey_overlay_title"
        | "hotkey-overlay-title"
        | "skip_confirmation"
        | "skip-confirmation"
        | "write_to_disk"
        | "write-to-disk"
        | "show_pointer"
        | "show-pointer"
        | "delay_ms"
        | "delay-ms"
        | "path" => None,
        _ => None,
    };
    Ok(action)
}

/// Parses an event handler value (`neori.on(event, ...)`) into an `Action`.
///
/// Handlers that are Lua functions are kept by the persistent script host and are not
/// part of the config; only string and table handlers end up here.
fn parse_startup_action(value: &Value) -> Result<Action, String> {
    match value {
        Value::String(s) => parse_action_string(&s.to_str().map_err(|e| format!("{e}"))?),
        Value::Table(t) => {
            for pair in t.clone().pairs::<Value, Value>() {
                let (ak, av) = pair.map_err(|e| format!("startup handler error: {e}"))?;
                let aks = key_str(&ak)?;
                if let Some(action) = action_from_table_key(t, &aks, &av)? {
                    return Ok(action);
                }
            }
            Err("startup handler table must contain an action".to_string())
        }
        _ => Err(format!(
            "startup handler must be a string or a table, got {}",
            value.type_name()
        )),
    }
}

fn parse_animations_section(table: LuaTable, config: &mut Config) -> Result<(), String> {
    let anims = &mut config.animations;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("animations error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => anims.off = value_is_truthy(&value),
            "slowdown" => {
                if let Ok(v) = value_to_f64(&value) {
                    anims.slowdown = v;
                }
            }
            "workspace_switch" | "workspace-switch" => {
                if let Some(a) = parse_animation_switch(&value, anims.workspace_switch.0)? {
                    anims.workspace_switch = WorkspaceSwitchAnim(a);
                }
            }
            "horizontal_view_movement" | "horizontal-view-movement" => {
                if let Some(a) = parse_animation_switch(&value, anims.horizontal_view_movement.0)? {
                    anims.horizontal_view_movement = HorizontalViewMovementAnim(a);
                }
            }
            "window_movement" | "window-movement" => {
                if let Some(a) = parse_animation_switch(&value, anims.window_movement.0)? {
                    anims.window_movement = WindowMovementAnim(a);
                }
            }
            "window_open" | "window-open" => {
                let mut shader = None;
                if let Some(a) =
                    parse_animation_switch_with_shader(&value, anims.window_open.anim, &mut shader)?
                {
                    anims.window_open.anim = a;
                }
                if shader.is_some() {
                    anims.window_open.custom_shader = shader;
                }
            }
            "window_close" | "window-close" => {
                let mut shader = None;
                if let Some(a) =
                    parse_animation_switch_with_shader(&value, anims.window_close.anim, &mut shader)?
                {
                    anims.window_close.anim = a;
                }
                if shader.is_some() {
                    anims.window_close.custom_shader = shader;
                }
            }
            "window_resize" | "window-resize" => {
                let mut shader = None;
                if let Some(a) =
                    parse_animation_switch_with_shader(&value, anims.window_resize.anim, &mut shader)?
                {
                    anims.window_resize.anim = a;
                }
                if shader.is_some() {
                    anims.window_resize.custom_shader = shader;
                }
            }
            "config_notification_open_close" | "config-notification-open-close" => {
                if let Some(a) =
                    parse_animation_switch(&value, anims.config_notification_open_close.0)?
                {
                    anims.config_notification_open_close = ConfigNotificationOpenCloseAnim(a);
                }
            }
            "exit_confirmation_open_close" | "exit-confirmation-open-close" => {
                if let Some(a) =
                    parse_animation_switch(&value, anims.exit_confirmation_open_close.0)?
                {
                    anims.exit_confirmation_open_close = ExitConfirmationOpenCloseAnim(a);
                }
            }
            "screenshot_ui_open" | "screenshot-ui-open" => {
                if let Some(a) = parse_animation_switch(&value, anims.screenshot_ui_open.0)? {
                    anims.screenshot_ui_open = ScreenshotUiOpenAnim(a);
                }
            }
            "overview_open_close" | "overview-open-close" => {
                if let Some(a) = parse_animation_switch(&value, anims.overview_open_close.0)? {
                    anims.overview_open_close = OverviewOpenCloseAnim(a);
                }
            }
            "recent_windows_close" | "recent-windows-close" => {
                if let Some(a) = parse_animation_switch(&value, anims.recent_windows_close.0)? {
                    anims.recent_windows_close = RecentWindowsCloseAnim(a);
                }
            }
            _ => unknown_key("animations", &ks)?,
        }
    }
    Ok(())
}

/// Parses a single animation value, accepting either a table of animation settings or a
/// bare boolean (`true` for on, `false` for off).
fn parse_animation_switch(value: &Value, default: Animation) -> Result<Option<Animation>, String> {
    parse_animation_switch_inner(value, default, None)
}

/// Like [`parse_animation_switch`], but also accepts a `custom_shader` key in the table form,
/// for the animations that support custom shaders.
fn parse_animation_switch_with_shader(
    value: &Value,
    default: Animation,
    shader: &mut Option<String>,
) -> Result<Option<Animation>, String> {
    parse_animation_switch_inner(value, default, Some(shader))
}

fn parse_animation_switch_inner(
    value: &Value,
    default: Animation,
    shader: Option<&mut Option<String>>,
) -> Result<Option<Animation>, String> {
    match value {
        Value::Table(t) => Ok(Some(parse_animation(t, default, shader)?)),
        Value::Boolean(on) => Ok(Some(Animation {
            off: !*on,
            kind: default.kind,
        })),
        Value::Nil => Ok(None),
        other => {
            warn!(
                "neori-config: animation value must be a table or boolean, got {}",
                other.type_name()
            );
            Ok(None)
        }
    }
}

fn parse_curve_value(value: &Value) -> Result<Curve, String> {
    let curve_str = match value {
        Value::Table(t) => {
            let items: Vec<Value> = t
                .clone()
                .sequence_values::<Value>()
                .collect::<Result<_, _>>()
                .map_err(|e| format!("{e}"))?;
            let name = items
                .first()
                .map(value_to_string)
                .transpose()?
                .unwrap_or_default();
            if name == "cubic-bezier" || name == "cubic_bezier" {
                if items.len() != 5 {
                    return Err("cubic-bezier requires 4 numbers".to_string());
                }
                let a = value_to_f64(&items[1])?;
                let b = value_to_f64(&items[2])?;
                let c = value_to_f64(&items[3])?;
                let d = value_to_f64(&items[4])?;
                return Ok(Curve::CubicBezier(a, b, c, d));
            }
            name
        }
        _ => value_to_string(value)?,
    };

    Ok(match curve_str.as_str() {
        "linear" => Curve::Linear,
        "ease-out-quad" | "ease_out_quad" => Curve::EaseOutQuad,
        "ease-out-cubic" | "ease_out_cubic" => Curve::EaseOutCubic,
        "ease-out-expo" | "ease_out_expo" => Curve::EaseOutExpo,
        _ => return Err(format!("unknown curve: {curve_str}")),
    })
}

/// Reads the xkb keymap file at `path` (relative to the config, `~` expanded), and watches it
/// so that changing it reloads the config. A file that can't be read is warned about.
fn read_xkb_file(path: &str) -> XkbFile {
    let path = match resolve_config_path(path) {
        Ok(path) => path,
        Err(err) => {
            warn!("failed to expand xkb file path: {err}");
            return XkbFile(None);
        }
    };

    watch_config_file(&path);
    match std::fs::read_to_string(&path) {
        Ok(keymap) => XkbFile(Some(keymap)),
        Err(err) => {
            warn!("failed to read xkb file from {path:?}: {err}");
            XkbFile(None)
        }
    }
}

/// Parses a `custom_shader` value: either the GLSL source as a string, or a
/// `{ path = "..." }` table pointing at a file with the source.
fn parse_custom_shader(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_str().map_err(|e| format!("custom_shader: {e}"))?.to_string()),
        Value::Table(t) => {
            let mut path = None;
            for pair in t.pairs::<Value, Value>() {
                let (key, value) = pair.map_err(|e| format!("custom_shader error: {e}"))?;
                let ks = key_str(&key)?;
                match ks.as_str() {
                    "path" => path = Some(value_to_string(&value)?),
                    _ => unknown_key("custom_shader", &ks)?,
                }
            }
            let path = path.ok_or_else(|| "custom shader path missing".to_string())?;
            let path = resolve_config_path(&path)?;
            // Watch the shader file even if reading fails, so that fixing it reloads the config.
            watch_config_file(&path);
            std::fs::read_to_string(&path)
                .map_err(|e| format!("failed to read custom shader from {path:?}: {e}"))
        }
        other => Err(format!(
            "custom_shader must be a string or a {{ path = ... }} table, got {}",
            other.type_name()
        )),
    }
}

fn parse_animation(
    table: &LuaTable,
    default: Animation,
    mut shader: Option<&mut Option<String>>,
) -> Result<Animation, String> {
    let mut off = false;
    let mut kind = default.kind;

    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("animation error: {e}"))?;
        let ks = key_str(&key)?;
        match ks.as_str() {
            "off" => off = value_is_truthy(&value),
            "spring" => {
                if let Value::Table(t) = &value {
                    let damping_ratio: f64 = t
                        .get("damping_ratio")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(1.0);
                    let stiffness: u32 = t
                        .get("stiffness")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(800);
                    let epsilon: f64 = t
                        .get("epsilon")
                        .map_err(|e| format!("{e}"))
                        .unwrap_or(0.0001);
                    kind = Kind::Spring(SpringParams {
                        damping_ratio,
                        stiffness,
                        epsilon,
                    });
                }
            }
            "duration_ms" | "duration-ms" => {
                if let Ok(ms) = value_to_u32(&value) {
                    if let Kind::Easing(params) = &mut kind {
                        params.duration_ms = ms;
                    } else {
                        kind = Kind::Easing(EasingParams {
                            duration_ms: ms,
                            curve: Curve::EaseOutCubic,
                        });
                    }
                }
            }
            "curve" => {
                let curve = parse_curve_value(&value)?;
                if let Kind::Easing(params) = &mut kind {
                    params.curve = curve;
                } else {
                    kind = Kind::Easing(EasingParams {
                        duration_ms: 250,
                        curve,
                    });
                }
            }
            "custom_shader" | "custom-shader" if shader.is_some() => {
                if let Some(shader) = shader.as_deref_mut() {
                    *shader = Some(parse_custom_shader(&value)?);
                }
            }
            _ => unknown_key("animation", &ks)?,
        }
    }

    Ok(Animation { off, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(lua: &Lua, key: &str) -> Bind {
        let action = Value::String(lua.create_string("close-window").unwrap());
        parse_bind_entry(key, action).unwrap()
    }

    #[test]
    fn recent_windows_bind_validation() {
        let lua = Lua::new();
        let ok = |key: &str, previous: &[&str]| {
            let previous: Vec<_> = previous.iter().map(|k| bind(&lua, k)).collect();
            validate_recent_windows_bind(key, &bind(&lua, key), &previous)
        };

        // A modifier other than the trigger is required.
        assert!(ok("Alt+Tab", &[]).is_ok());
        assert!(ok("Ctrl+Alt", &[]).is_ok());
        assert!(ok("Tab", &[]).unwrap_err().contains("must have a modifier"));
        assert!(ok("Alt", &[]).unwrap_err().contains("must have a modifier"));
        assert!(ok("Alt+Alt_L", &[]).unwrap_err().contains("must have a modifier"));

        // Only keyboard triggers are supported.
        assert!(ok("Alt+WheelScrollDown", &[])
            .unwrap_err()
            .contains("keyboard key"));

        // Binds triggered by the same key press conflict.
        assert!(ok("Ctrl+Alt_L", &["Ctrl+Alt"])
            .unwrap_err()
            .contains("conflicts"));
        assert!(ok("Ctrl+Alt", &["Alt+Tab"]).is_ok());
    }
}
