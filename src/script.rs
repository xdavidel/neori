//! Compositor-side glue for the Lua scripting runtime.
//!
//! Builds [`WindowSnapshot`]s and [`WindowCache`]s from live compositor state so the
//! Lua `neori.windows()` / `neori.focused_window()` queries and window event callbacks
//! see a consistent view of the world at each callback boundary.

use neori_config::{Config, Window as LuaWindow, WindowCache, WindowSnapshot};
use smithay::desktop::Window;

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
        let cache = build_window_cache(&self.neori);
        self.scripting.set_window_cache(cache);
        let window = LuaWindow(snapshot);
        let result = self.scripting.call_event_with(event, window);
        self.scripting.clear_window_cache();
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
    }

    /// Prime the window cache with the current compositor state so a Lua function bind
    /// can observe `neori.windows()` and `neori.focused_window()` at the moment of the
    /// key press.
    pub fn prime_window_cache(&mut self) {
        let cache = build_window_cache(&self.neori);
        self.scripting.set_window_cache(cache);
    }

    pub fn clear_window_cache(&mut self) {
        self.scripting.clear_window_cache();
    }
}

/// Helper so dispatcher code can clear the cache after a callback without naming the
/// State method directly.
#[allow(dead_code)]
pub fn clear(config: &Config, host: &neori_config::ScriptHost) {
    let _ = config;
    host.clear_window_cache();
}
