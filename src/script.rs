//! Compositor-side glue for the Lua scripting runtime.
//!
//! [`State`] implements [`ScriptContext`], so Lua reads compositor state live while a callback
//! runs, and carries out the changes Lua makes right away. Every callback goes through
//! [`State::run_lua`], which installs that context.
//!
//! Lua never runs inside another Lua callback, except for `window-opening`, whose result is
//! needed right away. An event that fires while a callback is running (for example because of a
//! change it made) goes into the [`LuaEventQueue`], and is delivered once the current callback is
//! done. Events
//! caused by a queued event are queued one level deeper; past [`MAX_LUA_EVENT_DEPTH`] levels they
//! are dropped, so handlers that keep triggering each other can't loop forever.

use std::collections::VecDeque;

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
use crate::layout::{Layout, LayoutElement};
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
///
/// The state is what the compositor has decided, which is what a change made from Lua affects
/// right away: `is_focused` follows the layout's focus (like `neori.focused_window()`), and
/// `is_fullscreen` is the state requested from the client, which applies it a little later.
pub fn window_snapshot(
    mapped: &Mapped,
    is_focused: bool,
    placement: Option<Placement>,
    output_name: Option<String>,
) -> WindowSnapshot {
    WindowSnapshot {
        id: mapped.id().get(),
        app_id: mapped.last_known_app_id().map(str::to_owned),
        title: mapped.last_known_title().map(str::to_owned),
        is_focused,
        // The window's own flag is only updated on the next layout refresh.
        is_floating: placement
            .as_ref()
            .map_or_else(|| mapped.is_floating(), |p| p.floating),
        is_fullscreen: mapped.pending_sizing_mode().is_fullscreen(),
        workspace_idx: placement.as_ref().map(|p| p.idx),
        workspace_name: placement.and_then(|p| p.name),
        output_name,
    }
}

/// Where a window is in the layout.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Global 1-based index of its workspace.
    idx: u8,
    /// Name of its workspace.
    name: Option<String>,
    /// Whether it's in the floating layout of its workspace.
    floating: bool,
}

/// Snapshot a single mapped window using the current layout to resolve workspace +
/// output. Convenient for the window-opened / window-closed fire sites.
pub fn snapshot_mapped(neori: &Neori, mapped: &Mapped) -> WindowSnapshot {
    let placement = placement_of(&neori.layout, mapped);
    let out_name = neori
        .layout
        .find_window_and_output(mapped.window.toplevel().expect("no x11").wl_surface())
        .and_then(|(_, output)| output.map(|o| o.name()));
    let is_focused = neori.layout.focus().is_some_and(|f| f.id() == mapped.id());
    window_snapshot(mapped, is_focused, placement, out_name)
}

/// Snapshots every mapped window, resolving its workspace (global 1-based index) and output
/// name.
pub fn window_snapshots(neori: &Neori) -> Vec<WindowSnapshot> {
    let focused = neori.layout.focus().map(Mapped::id);
    // Pre-build a reverse index window-id → (global workspace idx, workspace name)
    // in one pass over workspaces. The naive per-window version (which the
    // single-window `snapshot_mapped` still uses) walks every workspace per
    // lookup — O(windows × workspaces). This batch build is O(windows + workspaces).
    let mut ws_index: std::collections::HashMap<crate::window::mapped::MappedId, Placement> =
        std::collections::HashMap::new();
    let mut global_idx: u16 = 0;
    for (_mon, _, ws) in neori.layout.workspaces() {
        global_idx = global_idx.saturating_add(1);
        let idx = u8::try_from(global_idx).unwrap_or(u8::MAX);
        let name = ws.name().cloned();
        for window in ws.windows() {
            let placement = Placement {
                idx,
                name: name.clone(),
                floating: ws.is_floating(&window.window),
            };
            ws_index.insert(window.id(), placement);
        }
    }

    let mut windows: Vec<WindowSnapshot> = Vec::new();
    for (monitor, mapped) in neori.layout.windows() {
        let (placement, out_name) = match monitor {
            Some(mon) => (
                ws_index.get(&mapped.id()).cloned(),
                Some(mon.output().name()),
            ),
            None => (None, None),
        };
        let is_focused = focused == Some(mapped.id());
        windows.push(window_snapshot(mapped, is_focused, placement, out_name));
    }

    windows
}

/// Finds where `window` is in the layout, or `None` if it isn't on any workspace (e.g. a window
/// being destroyed).
fn placement_of(layout: &Layout<Mapped>, window: &Mapped) -> Option<Placement> {
    let window_handle: &Window = &window.window;
    (1u16..)
        .zip(layout.workspaces())
        .find(|(_, (_, _, ws))| ws.has_window(window_handle))
        .map(|(idx, (_, _, ws))| Placement {
            idx: u8::try_from(idx).unwrap_or(u8::MAX),
            name: ws.name().cloned(),
            floating: ws.is_floating(window_handle),
        })
}

/// How many levels deep an event may be caused by other events before it is dropped.
pub const MAX_LUA_EVENT_DEPTH: u32 = 16;

/// A Lua event waiting to be delivered.
struct DeferredLua {
    label: String,
    run: Box<dyn FnOnce() -> Result<(), String>>,
    allow_when_locked: bool,
    time: InputTime,
    depth: u32,
}

/// The Lua callback that is running.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Running {
    /// How many events deep this callback was caused by other events.
    pub depth: u32,
    /// Whether actions it carries out may run while the session is locked, like its bind.
    pub allow_when_locked: bool,
    /// The input time of the bind or event that started it.
    pub time: InputTime,
}

/// Lua events that fired while Lua was already running.
#[derive(Default)]
pub struct LuaEventQueue {
    /// The callback being delivered, or `None` when no Lua is running.
    current: Option<Running>,
    queue: VecDeque<DeferredLua>,
}

impl LuaEventQueue {
    /// Whether a Lua callback is running.
    pub fn is_running(&self) -> bool {
        self.current.is_some()
    }

    /// The depth of an event caused by the running callback.
    fn next_depth(&self) -> u32 {
        self.current.map_or(0, |r| r.depth + 1)
    }

    /// Queues an event fired while Lua is running, one level deeper than the current one.
    /// Returns the depth, or `Err` with the depth if it's too deep and was dropped.
    fn defer(&mut self, mut event: DeferredLua) -> Result<u32, u32> {
        let depth = self.next_depth();
        if depth > MAX_LUA_EVENT_DEPTH {
            return Err(depth);
        }
        event.depth = depth;
        self.queue.push_back(event);
        Ok(depth)
    }
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

    // Changes requested from Lua run right away, with the lock permission and input time of
    // the bind or event that started the callback. Lua events they cause are queued (see the
    // module docs), so Lua is never re-entered from here.

    fn do_action(&mut self, action: neori_config::Action) {
        let running = self.lua_events.current;
        let allow_when_locked = running.is_some_and(|r| r.allow_when_locked);
        let time = running.map_or_else(InputTime::now, |r| r.time);
        State::do_action(self, action, allow_when_locked, time);
    }

    fn output_action(&mut self, output: &str, action: neori_ipc::OutputAction) {
        self.apply_transient_output_config(output, action);
    }

    fn warp(&mut self, x: f64, y: f64) {
        self.move_cursor(smithay::utils::Point::from((x, y)));
    }

    fn device_op(&mut self, sysname: &str, op: DeviceOp) {
        apply_device_op(&mut self.neori, sysname, op);
    }
}

impl State {
    /// Runs a prepared Lua call with `self` as its [`ScriptContext`]. `allow_when_locked` and
    /// `time` apply to the actions the callback runs.
    ///
    /// `label` names the callback in error messages. Errors are warn!'d and flash the
    /// config-error notification; they do not propagate.
    ///
    /// If Lua is already running, the call is queued instead, and runs once the current callback
    /// is done (see the module docs).
    pub fn run_lua<A: IntoLuaMulti + Clone + 'static>(
        &mut self,
        label: &str,
        call: LuaCall,
        args: A,
        allow_when_locked: bool,
        time: InputTime,
    ) {
        let event = DeferredLua {
            label: label.to_owned(),
            run: Box::new(move || call.run(args)),
            allow_when_locked,
            time,
            depth: 0,
        };
        if self.lua_events.is_running() {
            if let Err(depth) = self.lua_events.defer(event) {
                warn!(
                    "Lua {label} callback dropped: Lua events caused each other {depth} levels \
                     deep; do handlers keep triggering each other?"
                );
                self.neori.config_error_notification.show();
            }
            return;
        }
        let running = Running {
            depth: 0,
            allow_when_locked,
            time,
        };
        self.with_lua_running(running, |state| state.deliver(event));
        self.deliver_queued_lua_events();
    }

    /// Runs `f` with Lua marked as `running`, so that events it fires are queued.
    pub(crate) fn with_lua_running<R>(
        &mut self,
        running: Running,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = self.lua_events.current.replace(running);
        let result = f(self);
        self.lua_events.current = outer;
        result
    }

    /// Runs a Lua event and reports its error, if any.
    fn deliver(&mut self, event: DeferredLua) {
        let result = with_script_context(self, event.run);
        self.report_lua_result(&event.label, result);
    }

    /// Delivers queued Lua events in order, including the ones they cause in turn.
    ///
    /// Only the outermost delivery does this; nested ones leave it to it.
    pub(crate) fn deliver_queued_lua_events(&mut self) {
        if self.lua_events.is_running() {
            return;
        }
        while let Some(event) = self.lua_events.queue.pop_front() {
            let running = Running {
                depth: event.depth,
                allow_when_locked: event.allow_when_locked,
                time: event.time,
            };
            self.with_lua_running(running, |state| state.deliver(event));
        }
    }

    /// Reports a Lua callback error: logged, and shown as the config error notification.
    fn report_lua_result(&mut self, label: &str, result: Result<(), String>) {
        if let Err(err) = result {
            warn!("Lua {label} callback error: {err}");
            self.neori.config_error_notification.show();
        }
    }

    /// Runs the Lua handlers for `event`, if there are any, including those connected to
    /// `target`, the object the event is about.
    fn run_lua_event<A: IntoLuaMulti + Clone + 'static>(
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
        // The window waits for the result, so this runs right away even if Lua is already
        // running. Events it causes are queued as usual.
        let running = Running {
            depth: self.lua_events.next_depth(),
            allow_when_locked: false,
            time: InputTime::now(),
        };
        let overrides = self.with_lua_running(running, |state| {
            state.scripting.begin_open_overrides();
            let result = with_script_context(state, || call.run(LuaWindow(snapshot)));
            let overrides = state.scripting.take_open_overrides();
            state.report_lua_result("window-opening", result);
            overrides
        });
        self.deliver_queued_lua_events();
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

/// Apply one `DeviceOp` requested from Lua to the matching libinput device, if any.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn event(label: &str) -> DeferredLua {
        DeferredLua {
            label: label.to_owned(),
            run: Box::new(|| Ok(())),
            allow_when_locked: false,
            time: InputTime::now(),
            depth: 0,
        }
    }

    fn running(depth: u32) -> Option<Running> {
        Some(Running {
            depth,
            allow_when_locked: false,
            time: InputTime::now(),
        })
    }

    #[test]
    fn queued_events_are_one_level_deeper_and_in_order() {
        let mut queue = LuaEventQueue::default();
        assert!(!queue.is_running());

        queue.current = running(0);
        assert_eq!(queue.defer(event("a")), Ok(1));
        assert_eq!(queue.defer(event("b")), Ok(1));
        queue.current = running(4);
        assert_eq!(queue.defer(event("c")), Ok(5));

        let order: Vec<_> = queue
            .queue
            .iter()
            .map(|e| (e.label.as_str(), e.depth))
            .collect();
        assert_eq!(order, [("a", 1), ("b", 1), ("c", 5)]);
    }

    #[test]
    fn too_deep_events_are_dropped() {
        let mut queue = LuaEventQueue {
            current: running(MAX_LUA_EVENT_DEPTH - 1),
            ..Default::default()
        };
        assert_eq!(queue.defer(event("last")), Ok(MAX_LUA_EVENT_DEPTH));
        queue.current = running(MAX_LUA_EVENT_DEPTH);
        assert_eq!(queue.defer(event("dropped")), Err(MAX_LUA_EVENT_DEPTH + 1));
        assert_eq!(queue.queue.len(), 1);
    }
}
