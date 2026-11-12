//! Compositor-side glue for the Lua scripting runtime.
//!
//! Builds [`WindowSnapshot`]s and [`WindowCache`]s from live compositor state so the
//! Lua `neori.windows()` / `neori.focused_window()` queries and window event callbacks
//! see a consistent view of the world at each callback boundary.

use neori_config::{
    Config, OutputCache, OutputSnapshot, Window as LuaWindow, WindowCache, WindowSnapshot,
};
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

    let mut windows: Vec<WindowSnapshot> = Vec::new();
    let mut focused_snapshot: Option<WindowSnapshot> = None;

    for (monitor, mapped) in neori.layout.windows() {
        let (ws_idx, ws_name, out_name) = match monitor {
            Some(mon) => {
                let output = mon.output();
                let out_name = Some(output.name());

                // Global workspace index: enumerate workspaces across all monitors in
                // the order the layout uses, and find this window's workspace.
                let ws_info = global_workspace_info_for_window(&neori.layout, mapped);
                (
                    ws_info.as_ref().map(|(idx, _)| *idx),
                    ws_info.as_ref().and_then(|(_, name)| name.clone()),
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
    /// Refreshes the window cache (so `neori.windows()` / `neori.focused_window()` see
    /// current state) and passes `snapshot` as the single arg to every registered
    /// `neori.on(event, function(w) … end)` handler. Any `neori.action` queued by the
    /// callbacks is drained and dispatched.
    ///
    /// Errors from a Lua callback are warn!'d and flash the config-error notification;
    /// they do not propagate.
    pub fn fire_window_event(&mut self, event: &str, snapshot: WindowSnapshot) {
        self.refresh_script_caches();
        let window = LuaWindow(snapshot);
        let result = self.scripting.call_event_with(event, window);
        self.scripting.clear_window_cache();
        self.scripting.clear_output_cache();
        self.drain_and_dispatch(event, result);
    }

    /// Refresh the ScriptHost's window + output caches from current compositor state.
    fn refresh_script_caches(&self) {
        let window_cache = build_window_cache(&self.neori);
        self.scripting.set_window_cache(window_cache);
        let ipc = self.backend.ipc_outputs();
        let ipc_outputs = ipc.lock().unwrap().clone();
        let output_cache = build_output_cache(&self.neori, &ipc_outputs);
        self.scripting.set_output_cache(output_cache);
    }

    /// Dispatch actions returned from the Lua call plus any pending output ops the
    /// callback enqueued via `Output` mutation methods. Surfaces Lua errors via the
    /// config-error notification.
    fn drain_and_dispatch(
        &mut self,
        event: &str,
        result: Result<Vec<neori_config::Action>, String>,
    ) {
        match result {
            Ok(actions) => {
                for action in actions {
                    self.do_action(action, false);
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
    }

    /// Prime the window + output caches with current compositor state so a Lua
    /// function bind can observe `neori.windows()`, `neori.focused_window()`,
    /// `neori.outputs()`, and `neori.focused_output()` at the moment of the key press.
    pub fn prime_window_cache(&mut self) {
        self.refresh_script_caches();
    }

    pub fn clear_window_cache(&mut self) {
        self.scripting.clear_window_cache();
        self.scripting.clear_output_cache();
    }

    /// Drain and dispatch any output ops enqueued by a bind callback via `Output`
    /// mutation methods. Mirror of the drain step inside `fire_window_event` so
    /// `dispatch_bind_inner` can call it alongside its action drain.
    pub fn drain_pending_output_ops_and_dispatch(&mut self) {
        for (output, op) in self.scripting.drain_pending_output_ops() {
            self.apply_transient_output_config(&output, op);
        }
    }
}

/// Helper so dispatcher code can clear the cache after a callback without naming the
/// State method directly.
#[allow(dead_code)]
pub fn clear(config: &Config, host: &neori_config::ScriptHost) {
    let _ = config;
    host.clear_window_cache();
    host.clear_output_cache();
}

/// Build a snapshot for one compositor output.
///
/// `ipc_outputs` carries the backend-maintained IPC-output mirror; the snapshot
/// reads VRR flags from it. Taking it as a parameter avoids needing access to
/// `State::backend` from here.
pub fn output_snapshot(
    neori: &Neori,
    output: &SmithayOutput,
    ipc_outputs: &std::collections::HashMap<crate::backend::OutputId, neori_ipc::Output>,
) -> OutputSnapshot {
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

    // VRR flags come from the IPC-output mirror. `off` is always false here because
    // only live outputs are in global_space; disabled outputs are gone from it.
    let name = output.name();
    let (vrr_supported, vrr_enabled) = ipc_outputs
        .values()
        .find(|o| o.name == name)
        .map(|o| (o.vrr_supported, o.vrr_enabled))
        .unwrap_or((false, false));
    // If the output is in the iteration (we're inside global_space.outputs()), it's on.
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
        let snap = output_snapshot(neori, output, ipc_outputs);
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
