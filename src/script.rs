//! Compositor-side glue for the Lua scripting runtime.
//!
//! [`State`] implements [`ScriptContext`], so Lua reads compositor state live while a callback
//! runs. Every callback goes through [`State::run_lua`], which installs that context and then
//! dispatches whatever the callback queued.

use neori_config::mlua::IntoLuaMulti;
use neori_config::parse_lua::LuaCall;
use neori_config::{
    with_script_context, DeviceOp, DeviceSnapshot, LuaOutput, LuaWorkspace, OutputSnapshot,
    PointerSnapshot, ScriptContext, SignalTarget, Window as LuaWindow, WindowSnapshot,
    WorkspaceSnapshot,
};
use smithay::backend::input::InputTime;
use smithay::desktop::Window;
use smithay::output::Output as SmithayOutput;

use crate::backend::VirtualOutputMarker;
use crate::layout::Layout;
use crate::neori::{Neori, State};
use crate::window::Mapped;

/// Build a snapshot of one mapped window.
///
/// Reads `app_id` / `title` from the mirrored fields on [`Mapped`] rather than the XDG
/// role, so they stay correct even when the client's unmap commit has already cleared
/// the role (notably, the `window-closed` Lua event fires in that window).
///
/// `global_workspace_idx` / `workspace_name` are optional; they are None when the
/// caller couldn't resolve the window's workspace (e.g. during destruction).
pub fn window_snapshot(
    mapped: &Mapped,
    global_workspace_idx: Option<u8>,
    workspace_name: Option<String>,
    output_name: Option<String>,
) -> WindowSnapshot {
    WindowSnapshot {
        id: mapped.id().get(),
        app_id: mapped.last_known_app_id().map(str::to_owned),
        title: mapped.last_known_title().map(str::to_owned),
        is_focused: mapped.is_focused(),
        is_floating: mapped.is_floating(),
        is_fullscreen: mapped.is_fullscreen(),
        workspace_idx: global_workspace_idx,
        workspace_name,
        output_name,
    }
}

/// Snapshot a single mapped window using the current layout to resolve workspace +
/// output. Convenient for the window-opened / window-closed fire sites.
pub fn snapshot_mapped(neori: &Neori, mapped: &Mapped) -> WindowSnapshot {
    let ws_info = global_workspace_info_for_window(&neori.layout, mapped);
    let out_name = neori
        .layout
        .find_window_and_output(mapped.window.toplevel().expect("no x11").wl_surface())
        .and_then(|(_, output)| output.map(|o| o.name()));
    window_snapshot(
        mapped,
        ws_info.as_ref().map(|(idx, _)| *idx),
        ws_info.as_ref().and_then(|(_, name)| name.clone()),
        out_name,
    )
}

/// Snapshots every mapped window, resolving its workspace (global 1-based index) and output
/// name.
pub fn window_snapshots(neori: &Neori) -> Vec<WindowSnapshot> {
    // Pre-build a reverse index window-id → (global workspace idx, workspace name)
    // in one pass over workspaces. The naive per-window version (which the
    // single-window `snapshot_mapped` still uses) walks every workspace per
    // lookup — O(windows × workspaces). This batch build is O(windows + workspaces).
    let mut ws_index: std::collections::HashMap<
        crate::window::mapped::MappedId,
        (u8, Option<String>),
    > = std::collections::HashMap::new();
    let mut global_idx: u16 = 0;
    for (_mon, _, ws) in neori.layout.workspaces() {
        global_idx = global_idx.saturating_add(1);
        let idx_u8 = u8::try_from(global_idx).unwrap_or(u8::MAX);
        let ws_name = ws.name().cloned();
        for window in ws.windows() {
            ws_index.insert(window.id(), (idx_u8, ws_name.clone()));
        }
    }

    let mut windows: Vec<WindowSnapshot> = Vec::new();
    for (monitor, mapped) in neori.layout.windows() {
        let (ws_idx, ws_name, out_name) = match monitor {
            Some(mon) => {
                let out_name = Some(mon.output().name());
                let info = ws_index.get(&mapped.id());
                (
                    info.map(|(idx, _)| *idx),
                    info.and_then(|(_, name)| name.clone()),
                    out_name,
                )
            }
            None => (None, None, None),
        };
        windows.push(window_snapshot(mapped, ws_idx, ws_name, out_name));
    }

    windows
}

/// Resolve the global 1-based workspace index and optional workspace name for the
/// workspace containing `window`. Returns None if the window isn't found in any
/// workspace (e.g. a window being destroyed mid-removal).
fn global_workspace_info_for_window(
    layout: &Layout<Mapped>,
    window: &Mapped,
) -> Option<(u8, Option<String>)> {
    let window_handle: &Window = &window.window;
    let mut idx: u16 = 0;
    for (_mon, _, ws) in layout.workspaces() {
        idx = idx.saturating_add(1);
        if ws.has_window(window_handle) {
            let as_u8 = u8::try_from(idx).unwrap_or(u8::MAX);
            return Some((as_u8, ws.name().cloned()));
        }
    }
    None
}

impl ScriptContext for State {
    fn windows(&mut self) -> Vec<WindowSnapshot> {
        window_snapshots(&self.neori)
    }

    fn window(&mut self, id: u64) -> Option<WindowSnapshot> {
        let (_, mapped) = self
            .neori
            .layout
            .windows()
            .find(|(_, m)| m.id().get() == id)?;
        Some(snapshot_mapped(&self.neori, mapped))
    }

    fn focused_window(&mut self) -> Option<WindowSnapshot> {
        let mapped = self.neori.layout.focus()?;
        Some(snapshot_mapped(&self.neori, mapped))
    }

    fn outputs(&mut self) -> Vec<OutputSnapshot> {
        output_snapshots(self)
    }

    fn output(&mut self, name: &str) -> Option<OutputSnapshot> {
        let output = self
            .neori
            .global_space
            .outputs()
            .find(|o| o.name() == name)?;
        let vrr = lookup_vrr(&self.backend, name);
        Some(output_snapshot(&self.neori, output, vrr))
    }

    fn focused_output(&mut self) -> Option<OutputSnapshot> {
        let name = self.neori.layout.active_output()?.name();
        self.output(&name)
    }

    fn workspaces(&mut self) -> Vec<WorkspaceSnapshot> {
        workspace_snapshots(&self.neori)
    }

    fn workspace(&mut self, id: u64) -> Option<WorkspaceSnapshot> {
        let focused_id = self.neori.layout.active_workspace_id().map(|id| id.get());
        let (idx, (monitor, _, ws)) = (1u16..)
            .zip(self.neori.layout.workspaces())
            .find(|(_, (_, _, ws))| ws.id().get() == id)?;
        Some(snapshot_workspace(monitor, idx, ws, focused_id))
    }

    fn focused_workspace(&mut self) -> Option<WorkspaceSnapshot> {
        let id = self.neori.layout.active_workspace_id()?;
        self.workspace(id.get())
    }

    fn pointer(&mut self) -> Option<PointerSnapshot> {
        pointer_snapshot(&self.neori)
    }

    fn devices(&mut self) -> Vec<DeviceSnapshot> {
        self.neori.devices.iter().map(device_snapshot).collect()
    }

    fn device(&mut self, sysname: &str) -> Option<DeviceSnapshot> {
        let device = self.neori.devices.iter().find(|d| d.sysname() == sysname)?;
        Some(device_snapshot(device))
    }

    fn keyboard_layout(&mut self) -> Option<String> {
        let keyboard = self.neori.seat.get_keyboard()?;
        Some(keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().unwrap();
            xkb.layout_name(xkb.active_layout()).to_owned()
        }))
    }
}

impl State {
    /// Runs a prepared Lua call with `self` as its [`ScriptContext`], then dispatches the
    /// actions, output ops, cursor warps and device ops it queued.
    ///
    /// `label` names the callback in error messages. Errors are warn!'d and flash the
    /// config-error notification; they do not propagate.
    pub fn run_lua<A: IntoLuaMulti + Clone>(
        &mut self,
        label: &str,
        call: LuaCall,
        args: A,
        allow_when_locked: bool,
        time: InputTime,
    ) {
        let result = self.call_lua(call, args);
        self.dispatch_lua_result(label, result, allow_when_locked, time);
    }

    /// Runs a prepared Lua call with `self` as its [`ScriptContext`], without dispatching
    /// anything it queued. Pair with [`State::dispatch_lua_result`].
    fn call_lua<A: IntoLuaMulti + Clone>(
        &mut self,
        call: LuaCall,
        args: A,
    ) -> Result<Vec<neori_config::Action>, String> {
        with_script_context(self, || call.run(args))
    }

    /// Dispatches the result of [`State::call_lua`] and everything else the callback queued.
    fn dispatch_lua_result(
        &mut self,
        label: &str,
        result: Result<Vec<neori_config::Action>, String>,
        allow_when_locked: bool,
        time: InputTime,
    ) {
        match result {
            Ok(actions) => {
                for action in actions {
                    self.do_action(action, allow_when_locked, time);
                }
            }
            Err(err) => {
                warn!("Lua {label} callback error: {err}");
                self.neori.config_error_notification.show();
            }
        }
        for (output, op) in self.scripting.drain_pending_output_ops() {
            self.apply_transient_output_config(&output, op);
        }
        for (x, y) in self.scripting.drain_pending_warps() {
            self.move_cursor(smithay::utils::Point::from((x, y)));
        }
        for (sysname, op) in self.scripting.drain_pending_device_ops() {
            apply_device_op(&mut self.neori, &sysname, op);
        }
    }

    /// Runs the Lua handlers for `event`, if there are any, including those connected to
    /// `target`, the object the event is about.
    fn run_lua_event<A: IntoLuaMulti + Clone>(
        &mut self,
        event: &str,
        target: Option<SignalTarget>,
        args: impl FnOnce(&mut Self) -> A,
    ) {
        if let Some(call) = self.scripting.event_call(event, target.as_ref()) {
            let args = args(self);
            self.run_lua(event, call, args, false, InputTime::now());
        }
    }

    /// Fire a window-lifecycle event to Lua callbacks.
    ///
    /// Costs nothing when no handler is registered.
    pub fn fire_window_event(&mut self, event: &str, snapshot: WindowSnapshot) {
        let target = SignalTarget::Window(snapshot.id);
        self.run_lua_event(event, Some(target), |_| LuaWindow(snapshot));
    }

    /// Fire the `window-opening` event just before placement.
    ///
    /// Opens the open-overrides guard, invokes the Lua handler with a
    /// pre-placement `Window` snapshot (id=0 — the window has no stable
    /// MappedId yet), then takes back the accumulated overrides so the caller
    /// (`send_initial_configure`) can merge them into `ResolvedWindowRules`.
    pub fn fire_window_opening_event(
        &mut self,
        snapshot: WindowSnapshot,
    ) -> neori_config::OpenOverrides {
        // The window has no id yet, so it can't have signal handlers of its own.
        let Some(call) = self.scripting.event_call("window-opening", None) else {
            return neori_config::OpenOverrides::default();
        };
        self.scripting.begin_open_overrides();
        let result = self.call_lua(call, LuaWindow(snapshot));
        // Take the overrides before dispatching: an action could open another window and
        // re-enter this function.
        let overrides = self.scripting.take_open_overrides();
        self.dispatch_lua_result("window-opening", result, false, InputTime::now());
        overrides
    }

    /// Fire a workspace-lifecycle event (`workspace-focused`, `workspace-added`,
    /// `workspace-removed`) to Lua callbacks. Mirrors [`State::fire_window_event`].
    pub fn fire_workspace_event(&mut self, event: &str, snapshot: WorkspaceSnapshot) {
        let target = SignalTarget::Workspace(snapshot.id);
        self.run_lua_event(event, Some(target), |_| LuaWorkspace(snapshot));
    }

    /// Fire the `output-focused` event with an `Output` userdata built from the
    /// output named `output_name`. Falls back to a minimal snapshot (just the name)
    /// if the output is no longer in global_space.
    pub fn fire_output_focused_event(&mut self, output_name: String) {
        let target = SignalTarget::Output(output_name.clone());
        self.run_lua_event("output-focused", Some(target), |state| {
            let vrr = lookup_vrr(&state.backend, &output_name);
            let snapshot = state
                .neori
                .global_space
                .outputs()
                .find(|o| o.name() == output_name)
                .map(|o| output_snapshot(&state.neori, o, vrr))
                .unwrap_or_else(|| OutputSnapshot {
                    name: output_name,
                    ..Default::default()
                });
            LuaOutput(snapshot)
        });
    }

    /// Fire the Lua `layout-changed` event with the new active keyboard layout
    /// name. Called from `ipc_refresh_keyboard_layout_index` right after the
    /// IPC-side diff detects the switch.
    pub fn fire_layout_changed_event(&mut self, name: String) {
        self.run_lua_event("layout-changed", None, |_| name);
    }
}

/// Look up `(vrr_supported, vrr_enabled)` for `output_name` in the backend's
/// IPC-output mirror. Holds the lock just long enough to read the two bools
/// — critical because the IPC server thread reads the same mirror.
pub fn lookup_vrr(backend: &crate::backend::Backend, output_name: &str) -> (bool, bool) {
    let ipc = backend.ipc_outputs();
    let guard = ipc.lock().unwrap();
    guard
        .values()
        .find(|o| o.name == output_name)
        .map(|o| (o.vrr_supported, o.vrr_enabled))
        .unwrap_or((false, false))
}

/// Build a snapshot for one compositor output.
///
/// `vrr` is `(vrr_supported, vrr_enabled)`; callers either pass [`lookup_vrr`]
/// for a single-output path or iterate the full IPC mirror once for a bulk
/// build (`output_snapshots`).
pub fn output_snapshot(neori: &Neori, output: &SmithayOutput, vrr: (bool, bool)) -> OutputSnapshot {
    let props = output.physical_properties();
    let serial = if props.serial_number.is_empty() {
        None
    } else {
        Some(props.serial_number.clone())
    };
    let make = if props.make.is_empty() {
        None
    } else {
        Some(props.make.clone())
    };
    let model = if props.model.is_empty() {
        None
    } else {
        Some(props.model.clone())
    };

    let mode = output.current_mode().map(|m| {
        let hz = m.refresh as f64 / 1000.0;
        format!("{}x{}@{:.3}", m.size.w, m.size.h, hz)
    });
    let modes: Vec<String> = output
        .modes()
        .into_iter()
        .map(|m| {
            let hz = m.refresh as f64 / 1000.0;
            format!("{}x{}@{:.3}", m.size.w, m.size.h, hz)
        })
        .collect();
    let scale = Some(output.current_scale().fractional_scale());
    let transform = Some(smithay_transform_name(output.current_transform()).to_string());

    let position = neori
        .global_space
        .output_geometry(output)
        .map(|r| (r.loc.x, r.loc.y));

    let is_focused = neori.layout.active_output() == Some(output);
    let is_virtual = VirtualOutputMarker::is_virtual(output);

    // VRR flags come from the IPC-output mirror (passed in). `off` is always
    // false here because only live outputs are in global_space; disabled
    // outputs are gone from it.
    let name = output.name();
    let (vrr_supported, vrr_enabled) = vrr;
    let off = false;

    OutputSnapshot {
        name,
        make,
        model,
        serial,
        mode,
        modes,
        scale,
        transform,
        position,
        is_focused,
        is_virtual,
        off,
        vrr_supported,
        vrr_enabled,
    }
}

/// Snapshots every output in the global space.
fn output_snapshots(state: &State) -> Vec<OutputSnapshot> {
    let ipc = state.backend.ipc_outputs();
    let ipc_outputs = ipc.lock().unwrap().clone();
    state
        .neori
        .global_space
        .outputs()
        .map(|output| {
            let name = output.name();
            let vrr = ipc_outputs
                .values()
                .find(|o| o.name == name)
                .map(|o| (o.vrr_supported, o.vrr_enabled))
                .unwrap_or((false, false));
            output_snapshot(&state.neori, output, vrr)
        })
        .collect()
}

/// Build a single [`WorkspaceSnapshot`] for the given workspace. Caller supplies
/// the 1-based global index (position in `layout.workspaces()` order) because
/// that's the semantic Lua `ws:idx()` has always had, and it isn't recoverable
/// from the workspace alone.
pub fn snapshot_workspace(
    monitor: Option<&crate::layout::monitor::Monitor<crate::window::Mapped>>,
    global_idx: u16,
    ws: &crate::layout::workspace::Workspace<crate::window::Mapped>,
    focused_ws_id: Option<u64>,
) -> WorkspaceSnapshot {
    let id = ws.id().get();
    WorkspaceSnapshot {
        idx: u8::try_from(global_idx).unwrap_or(u8::MAX),
        name: ws.name().cloned(),
        output_name: monitor.map(|m| m.output().name()),
        is_focused: focused_ws_id == Some(id),
        id,
        window_ids: ws.windows().map(|w| w.id().get()).collect(),
    }
}

/// Snapshots every workspace, in global 1-based order (the order `Layout::workspaces()`
/// yields).
fn workspace_snapshots(neori: &Neori) -> Vec<WorkspaceSnapshot> {
    let focused_id = neori.layout.active_workspace_id().map(|id| id.get());
    (1u16..)
        .zip(neori.layout.workspaces())
        .map(|(idx, (monitor, _, ws))| snapshot_workspace(monitor, idx, ws, focused_id))
        .collect()
}

/// Snapshots one libinput device for Lua.
///
/// Touchpad detection matches `apply_libinput_settings` (non-zero `tap_finger_count`).
fn device_snapshot(device: &smithay::reexports::input::Device) -> DeviceSnapshot {
    use smithay::reexports::input::DeviceCapability;
    DeviceSnapshot {
        name: device.name().into_owned(),
        sysname: device.sysname().to_owned(),
        product_id: device.id_product(),
        vendor_id: device.id_vendor(),
        is_pointer: device.has_capability(DeviceCapability::Pointer),
        is_keyboard: device.has_capability(DeviceCapability::Keyboard),
        is_touch: device.has_capability(DeviceCapability::Touch),
        is_touchpad: device.config_tap_finger_count() > 0,
    }
}

/// Apply one Lua-enqueued `DeviceOp` to the matching libinput device, if any.
///
/// Mutations land on the device directly; they persist until the next
/// `apply_libinput_settings` (config reload) overwrites them.
pub fn apply_device_op(neori: &mut Neori, sysname: &str, op: DeviceOp) {
    use smithay::reexports::input::SendEventsMode;
    let mut target: Option<smithay::reexports::input::Device> = None;
    for device in &neori.devices {
        if device.sysname() == sysname {
            target = Some(device.clone());
            break;
        }
    }
    let Some(mut device) = target else {
        return;
    };
    match op {
        DeviceOp::SetTap(enable) => {
            let _ = device.config_tap_set_enabled(enable);
        }
        DeviceOp::SetNaturalScroll(enable) => {
            let _ = device.config_scroll_set_natural_scroll_enabled(enable);
        }
        DeviceOp::SetAccel(speed) => {
            let _ = device.config_accel_set_speed(speed);
        }
        DeviceOp::SetLeftHanded(enable) => {
            let _ = device.config_left_handed_set(enable);
        }
        DeviceOp::SetDisabled(disabled) => {
            let _ = device.config_send_events_set_mode(if disabled {
                SendEventsMode::DISABLED
            } else {
                SendEventsMode::ENABLED
            });
        }
    }
}

/// Snapshot the current pointer position and the output it sits on.
///
/// Returns `None` if no seat pointer exists (shouldn't happen on a running
/// compositor, but keeps the signature total for callers).
fn pointer_snapshot(neori: &Neori) -> Option<PointerSnapshot> {
    let pointer = neori.seat.get_pointer()?;
    let loc = pointer.current_location();
    let output_name = neori
        .global_space
        .outputs()
        .find(|o| {
            neori
                .global_space
                .output_geometry(o)
                .map(|geo| geo.to_f64().contains(loc))
                .unwrap_or(false)
        })
        .map(|o| o.name());
    Some(PointerSnapshot {
        x: loc.x,
        y: loc.y,
        output_name,
    })
}

/// Convert a Smithay output transform into the string form the Lua `Output:transform()`
/// accessor exposes (and that `set_transform(…)` parses back).
fn smithay_transform_name(t: smithay::utils::Transform) -> &'static str {
    use smithay::utils::Transform;
    match t {
        Transform::Normal => "normal",
        Transform::_90 => "90",
        Transform::_180 => "180",
        Transform::_270 => "270",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped-90",
        Transform::Flipped180 => "flipped-180",
        Transform::Flipped270 => "flipped-270",
    }
}
