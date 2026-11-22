//! Compositor-side glue for the Lua scripting runtime.
//!
//! Builds [`WindowSnapshot`]s and [`WindowCache`]s from live compositor state so the
//! Lua `neori.windows()` / `neori.focused_window()` queries and window event callbacks
//! see a consistent view of the world at each callback boundary.

use neori_config::{
    Config, DeviceOp, LuaOutput, LuaWorkspace, OutputCache, OutputSnapshot, PointerSnapshot,
    Window as LuaWindow, WindowCache, WindowSnapshot, WorkspaceCache, WorkspaceSnapshot,
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

/// Build the full window cache from the current compositor state.
///
/// Walks every mapped window, resolves its workspace (global 1-based index) and output
/// name, and records which one is currently focused.
pub fn build_window_cache(neori: &Neori) -> WindowCache {
    let focused_mapped = neori.layout.focus().map(Mapped::id);

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
    let mut focused_snapshot: Option<WindowSnapshot> = None;

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
        let snap = window_snapshot(mapped, ws_idx, ws_name, out_name);
        if focused_mapped == Some(mapped.id()) {
            focused_snapshot = Some(snap.clone());
        }
        windows.push(snap);
    }

    WindowCache {
        windows,
        focused: focused_snapshot,
    }
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

impl State {
    /// Fire a window-lifecycle event to Lua callbacks.
    ///
    /// Returns early via `has_event_handlers` when no handler is registered —
    /// no cache refresh, no snapshot cost. Otherwise refreshes the Lua caches
    /// (so `neori.windows()` / `neori.focused_window()` see current state),
    /// passes `snapshot` as the single arg to every registered
    /// `neori.on(event, function(w) … end)` handler, and drains any
    /// `neori.action` queued by the callbacks.
    ///
    /// Errors from a Lua callback are warn!'d and flash the config-error notification;
    /// they do not propagate.
    pub fn fire_window_event(&mut self, event: &str, snapshot: WindowSnapshot) {
        if !self.scripting.has_event_handlers(event) {
            return;
        }
        self.refresh_script_caches();
        let window = LuaWindow(snapshot);
        let result = self.scripting.call_event_with(event, window);
        self.scripting.clear_script_caches();
        self.drain_and_dispatch(event, result);
    }

    /// Refresh every Lua-facing cache (window, output, workspace, pointer,
    /// device) from current compositor state. Fire helpers gate the call on
    /// `has_event_handlers` first — no event handler listening means no cache
    /// rebuild.
    fn refresh_script_caches(&self) {
        let window_cache = build_window_cache(&self.neori);
        self.scripting.set_window_cache(window_cache);
        let ipc = self.backend.ipc_outputs();
        let ipc_outputs = ipc.lock().unwrap().clone();
        let output_cache = build_output_cache(&self.neori, &ipc_outputs);
        self.scripting.set_output_cache(output_cache);
        let workspace_cache = build_workspace_cache(&self.neori);
        self.scripting.set_workspace_cache(workspace_cache);
        if let Some(snapshot) = build_pointer_snapshot(&self.neori) {
            self.scripting.set_pointer_cache(snapshot);
        } else {
            self.scripting.clear_pointer_cache();
        }
        self.scripting
            .set_device_cache(build_device_cache(&self.neori));
    }

    /// Dispatch actions returned from the Lua call plus any pending output ops the
    /// callback enqueued via `Output` mutation methods, and any pending cursor
    /// warps from `neori.warp(x, y)`. Surfaces Lua errors via the
    /// config-error notification.
    fn drain_and_dispatch(
        &mut self,
        event: &str,
        result: Result<Vec<neori_config::Action>, String>,
    ) {
        match result {
            Ok(actions) => {
                for action in actions {
                    self.do_action(action, false, InputTime::now());
                }
            }
            Err(err) => {
                warn!("Lua {event} callback error: {err}");
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

    /// Prime the window + output caches with current compositor state so a Lua
    /// function bind can observe `neori.windows()`, `neori.focused_window()`,
    /// `neori.outputs()`, and `neori.focused_output()` at the moment of the key press.
    pub fn prime_window_cache(&mut self) {
        self.refresh_script_caches();
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
        if !self.scripting.has_event_handlers("window-opening") {
            return neori_config::OpenOverrides::default();
        }
        self.refresh_script_caches();
        self.scripting.begin_open_overrides();
        let window = LuaWindow(snapshot);
        let result = self.scripting.call_event_with("window-opening", window);
        let overrides = self.scripting.take_open_overrides();
        self.scripting.clear_script_caches();
        self.drain_and_dispatch("window-opening", result);
        overrides
    }

    /// Fire a workspace-lifecycle event (`workspace-focused`, `workspace-added`,
    /// `workspace-removed`) to Lua callbacks. Mirrors [`State::fire_window_event`].
    pub fn fire_workspace_event(&mut self, event: &str, snapshot: WorkspaceSnapshot) {
        if !self.scripting.has_event_handlers(event) {
            return;
        }
        self.refresh_script_caches();
        let ws = LuaWorkspace(snapshot);
        let result = self.scripting.call_event_with(event, ws);
        self.scripting.clear_script_caches();
        self.drain_and_dispatch(event, result);
    }

    /// Fire the `output-focused` event with an `Output` userdata built from the
    /// output named `output_name`. Falls back to a minimal snapshot (just the name)
    /// if the output is no longer in global_space.
    pub fn fire_output_focused_event(&mut self, output_name: String) {
        if !self.scripting.has_event_handlers("output-focused") {
            return;
        }
        self.refresh_script_caches();
        let vrr = lookup_vrr(&self.backend, &output_name);
        let snapshot = self
            .neori
            .global_space
            .outputs()
            .find(|o| o.name() == output_name)
            .map(|o| output_snapshot(&self.neori, o, vrr))
            .unwrap_or_else(|| OutputSnapshot {
                name: output_name,
                ..Default::default()
            });
        let result = self
            .scripting
            .call_event_with("output-focused", LuaOutput(snapshot));
        self.scripting.clear_script_caches();
        self.drain_and_dispatch("output-focused", result);
    }

    /// Fire the Lua `layout-changed` event with the new active keyboard layout
    /// name. Called from `ipc_refresh_keyboard_layout_index` right after the
    /// IPC-side diff detects the switch.
    pub fn fire_layout_changed_event(&mut self, name: String) {
        if !self.scripting.has_event_handlers("layout-changed") {
            return;
        }
        self.refresh_script_caches();
        let result = self.scripting.call_event_with("layout-changed", name);
        self.scripting.clear_script_caches();
        self.drain_and_dispatch("layout-changed", result);
    }

    /// Clear every Lua-facing cache on the ScriptHost. Mirrors
    /// [`neori_config::ScriptHost::clear_script_caches`]. Called by the input
    /// dispatcher after a bind callback returns so the primed caches from
    /// `prime_window_cache` don't outlive the callback.
    pub fn clear_script_caches(&mut self) {
        self.scripting.clear_script_caches();
    }

    /// Drain and dispatch any output ops / cursor warps enqueued by a bind
    /// callback via `Output` mutation methods or `neori.warp`. Mirror of the
    /// drain step inside `fire_window_event` so `dispatch_bind_inner` can call
    /// it alongside its action drain.
    pub fn drain_pending_output_ops_and_dispatch(&mut self) {
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
}

/// Helper so dispatcher code can clear the Lua caches after a callback without
/// naming the State method directly.
#[allow(dead_code)]
pub fn clear(config: &Config, host: &neori_config::ScriptHost) {
    let _ = config;
    host.clear_script_caches();
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
/// build (`build_output_cache`).
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

/// Build the full output cache from the current compositor state.
pub fn build_output_cache(
    neori: &Neori,
    ipc_outputs: &std::collections::HashMap<crate::backend::OutputId, neori_ipc::Output>,
) -> OutputCache {
    let focused_output = neori.layout.active_output().cloned();
    let mut outputs: Vec<OutputSnapshot> = Vec::new();
    let mut focused_snapshot: Option<OutputSnapshot> = None;
    for output in neori.global_space.outputs() {
        let name = output.name();
        let vrr = ipc_outputs
            .values()
            .find(|o| o.name == name)
            .map(|o| (o.vrr_supported, o.vrr_enabled))
            .unwrap_or((false, false));
        let snap = output_snapshot(neori, output, vrr);
        if focused_output.as_ref() == Some(output) {
            focused_snapshot = Some(snap.clone());
        }
        outputs.push(snap);
    }
    OutputCache {
        outputs,
        focused: focused_snapshot,
    }
}

/// Build the full workspace cache from the current compositor state.
///
/// Walks every workspace in global 1-based order (the same order `Layout::workspaces()`
/// yields), records the workspace's id, optional name, owning output name, the
/// MappedIds of its windows, and whether it is the globally focused workspace.
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

pub fn build_workspace_cache(neori: &Neori) -> WorkspaceCache {
    let focused_id = neori.layout.active_workspace_id();
    let mut workspaces: Vec<WorkspaceSnapshot> = Vec::new();
    let mut focused_snapshot: Option<WorkspaceSnapshot> = None;

    let mut global_idx: u16 = 0;
    for (monitor, _per_mon_idx, ws) in neori.layout.workspaces() {
        global_idx = global_idx.saturating_add(1);
        let idx_u8 = u8::try_from(global_idx).unwrap_or(u8::MAX);
        let output_name = monitor.map(|m| m.output().name());
        let id = ws.id().get();
        let is_focused = focused_id == Some(ws.id());
        let window_ids: Vec<u64> = ws.windows().map(|w| w.id().get()).collect();
        let snap = WorkspaceSnapshot {
            idx: idx_u8,
            name: ws.name().cloned(),
            output_name,
            is_focused,
            id,
            window_ids,
        };
        if is_focused {
            focused_snapshot = Some(snap.clone());
        }
        workspaces.push(snap);
    }

    WorkspaceCache {
        workspaces,
        focused: focused_snapshot,
    }
}

/// Snapshot connected input devices for Lua.
///
/// Walks `Neori::devices` (the set of libinput devices the compositor is
/// tracking) and reads each device's metadata and capability bits. Touchpad
/// detection matches `apply_libinput_settings` (non-zero `tap_finger_count`).
pub fn build_device_cache(neori: &Neori) -> neori_config::DeviceCache {
    use smithay::reexports::input::DeviceCapability;
    let mut devices = Vec::new();
    for device in &neori.devices {
        let is_touchpad = device.config_tap_finger_count() > 0;
        let is_pointer = device.has_capability(DeviceCapability::Pointer);
        let is_keyboard = device.has_capability(DeviceCapability::Keyboard);
        let is_touch = device.has_capability(DeviceCapability::Touch);
        devices.push(neori_config::DeviceSnapshot {
            name: device.name().into_owned(),
            sysname: device.sysname().to_owned(),
            product_id: device.id_product(),
            vendor_id: device.id_vendor(),
            is_pointer,
            is_keyboard,
            is_touch,
            is_touchpad,
        });
    }
    neori_config::DeviceCache { devices }
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
pub fn build_pointer_snapshot(neori: &Neori) -> Option<PointerSnapshot> {
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
