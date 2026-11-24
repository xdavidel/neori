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
//!
//! Some events are found by comparing the state at each refresh instead (`window-focused`, the
//! property signals, …), so they fire after the callback that caused them returned. A change made
//! from Lua is remembered with its depth, and those events fire one level deeper than it during
//! the next refresh ([`LUA_CAUSED_SIGNALS`]). Handlers that keep undoing each other across
//! refreshes hit the same limit.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use neori_config::mlua::IntoLuaMulti;
use neori_config::parse_lua::LuaCall;
use neori_config::{
    with_script_context, DeviceOp, DeviceSnapshot, LuaDevice, LuaOutput, LuaTimer, LuaWorkspace,
    OutputSnapshot, PointerSnapshot, PropertyValue, ScriptContext, SignalTarget,
    Window as LuaWindow, WindowSnapshot, WorkspaceSnapshot,
};
use smithay::backend::input::InputTime;
use smithay::desktop::Window;
use smithay::input::pointer::{
    CursorIcon, CursorImageStatus, Focus, GrabStartData as PointerGrabStartData,
};
use smithay::output::Output as SmithayOutput;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::utils::SERIAL_COUNTER;

use crate::backend::VirtualOutputMarker;
use crate::input::AnyStartData;
use crate::layout::{Layout, LayoutElement};
use crate::neori::{Neori, State};
use crate::utils::ResizeEdge;
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
    xwayland_pid: Option<i32>,
) -> WindowSnapshot {
    // X11 windows all belong to xwayland-satellite, so its pid says nothing about the window.
    let pid = mapped
        .credentials()
        .map(|c| c.pid)
        .filter(|&pid| Some(pid) != xwayland_pid);
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
        is_urgent: mapped.is_urgent(),
        is_maximized: mapped.pending_sizing_mode().is_maximized(),
        pid,
        workspace_idx: placement.as_ref().map(|p| p.idx),
        workspace_name: placement.and_then(|p| p.name),
        output_name,
        opacity: Some(mapped.effective_opacity()).filter(|&o| o < 1.),
    }
}

/// Parses `"top"`, `"bottom-right"` and so on.
fn parse_resize_edges(edges: &str) -> Result<Option<ResizeEdge>, String> {
    let edges = match edges {
        "top" => ResizeEdge::TOP,
        "bottom" => ResizeEdge::BOTTOM,
        "left" => ResizeEdge::LEFT,
        "right" => ResizeEdge::RIGHT,
        "top-left" => ResizeEdge::TOP_LEFT,
        "top-right" => ResizeEdge::TOP_RIGHT,
        "bottom-left" => ResizeEdge::BOTTOM_LEFT,
        "bottom-right" => ResizeEdge::BOTTOM_RIGHT,
        _ => {
            return Err(format!(
                "unknown edges {edges:?}: use \"top\", \"bottom\", \"left\", \"right\", \
                 \"top-left\", \"top-right\", \"bottom-left\" or \"bottom-right\""
            ))
        }
    };
    Ok(Some(edges))
}

impl State {
    /// Checks that a move or resize of the window with this id can start from Lua, and returns
    /// the window, the held mouse button and the pointer location.
    fn prepare_window_grab(
        &mut self,
        id: u64,
    ) -> Result<
        (
            smithay::desktop::Window,
            u32,
            smithay::utils::Point<f64, smithay::utils::Logical>,
        ),
        String,
    > {
        if self.neori.is_locked() {
            return Err("the screen is locked".to_owned());
        }
        let window = self
            .neori
            .layout
            .windows()
            .find(|(_, m)| m.id().get() == id)
            .map(|(_, m)| m.window.clone())
            .ok_or("this window is gone")?;
        let pointer = self.neori.seat.get_pointer().unwrap();
        // While a button is held over a client, the click grab sends it the pointer events;
        // a move or resize may take over from it, but not from a grab of neori's own.
        let other_grab = pointer
            .with_grab(|_, grab| {
                !grab
                    .as_any()
                    .is::<crate::input::click_grab::ClickGrab<State>>()
            })
            .unwrap_or(false);
        if other_grab {
            return Err("the pointer is already moving or resizing something".to_owned());
        }
        let button = self.neori.pointer_button_in_flight.ok_or(
            "needs a held mouse button: call it from a mouse bind or a pointer-button handler",
        )?;
        // A bind suppressed this button: let its release through, so it ends the grab. Its release
        // action, if any, doesn't run then.
        if self.neori.suppressed_buttons.remove(&button) {
            self.neori.pending_mouse_release_binds.remove(&button);
        }
        Ok((window, button, pointer.current_location()))
    }
}

/// Ends process `pid` with SIGKILL, refusing pid 1 and neori itself.
fn kill_process(pid: i32) -> Result<(), String> {
    if pid <= 1 || u32::try_from(pid).ok() == Some(std::process::id()) {
        return Err(format!("refusing to kill process {pid}"));
    }
    // SAFETY: kill() has no memory safety requirements.
    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("error killing process {pid}: {err}"));
    }
    Ok(())
}

/// Snapshots a mapped layer-shell surface on `output`.
pub fn layer_snapshot(
    mapped: &crate::layer::MappedLayer,
    output: Option<&SmithayOutput>,
) -> neori_config::LayerSnapshot {
    use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};
    let layer = mapped.surface();
    let kind = match layer.layer() {
        Layer::Background => "background",
        Layer::Bottom => "bottom",
        Layer::Top => "top",
        Layer::Overlay => "overlay",
    };
    let keyboard_interactivity = match layer.cached_state().keyboard_interactivity {
        KeyboardInteractivity::None => "none",
        KeyboardInteractivity::Exclusive => "exclusive",
        KeyboardInteractivity::OnDemand => "on-demand",
    };
    neori_config::LayerSnapshot {
        id: mapped.id(),
        namespace: layer.namespace().to_owned(),
        layer: kind.to_owned(),
        output_name: output.map(|o| o.name()),
        pid: crate::utils::get_credentials_for_surface(layer.wl_surface()).map(|c| c.pid),
        keyboard_interactivity: keyboard_interactivity.to_owned(),
    }
}

/// Snapshots every mapped layer-shell surface, output by output.
fn layer_snapshots(neori: &Neori) -> Vec<neori_config::LayerSnapshot> {
    let mut layers = Vec::new();
    for output in neori.layout.outputs() {
        let map = smithay::desktop::layer_map_for_output(output);
        for layer in map.layers() {
            if let Some(mapped) = neori.mapped_layer_surfaces.get(layer) {
                layers.push(layer_snapshot(mapped, Some(output)));
            }
        }
    }
    layers
}

/// The mapped layer-shell surface with this id.
fn find_layer(neori: &Neori, id: u64) -> Option<smithay::desktop::LayerSurface> {
    neori
        .mapped_layer_surfaces
        .iter()
        .find(|(_, mapped)| mapped.id() == id)
        .map(|(layer, _)| layer.clone())
}

/// The pid of xwayland-satellite, which all X11 windows belong to, if it runs.
fn xwayland_pid(neori: &Neori) -> Option<i32> {
    neori.satellite.as_ref().and_then(|s| s.pid())
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
    window_snapshot(mapped, is_focused, placement, out_name, xwayland_pid(neori))
}

/// Snapshots every mapped window, resolving its workspace (global 1-based index) and output
/// name.
pub fn window_snapshots(neori: &Neori) -> Vec<WindowSnapshot> {
    let xwayland_pid = xwayland_pid(neori);
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
        windows.push(window_snapshot(
            mapped,
            is_focused,
            placement,
            out_name,
            xwayland_pid,
        ));
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

/// The events found by comparing the state at each refresh that a change made from Lua can
/// cause. During the refresh after such a change, they continue its depth (see the module docs).
/// Title and urgency changes come only from clients, so they always start at depth 0, and a
/// handler that reacts to a title changing on every frame isn't mistaken for a loop.
const LUA_CAUSED_SIGNALS: &[&str] = &[
    "window-focused",
    "workspace-focused",
    "output-focused",
    "window-fullscreen-changed",
    "window-floating-changed",
    "window-workspace-changed",
    "workspace-added",
    "workspace-removed",
    "workspace-name-changed",
    "output-mode-changed",
    "layout-changed",
];

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
    /// The depth of an event caused by a change Lua made since the current refresh started:
    /// one more than the deepest callback that made one.
    changed: Option<u32>,
    /// `changed` as it was when the current refresh started, for the changes Lua made before it.
    carried: Option<u32>,
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

    /// Remembers that the running callback changed the compositor state.
    fn note_change(&mut self) {
        let depth = self.next_depth().max(1);
        self.changed = Some(self.changed.map_or(depth, |d| d.max(depth)));
    }

    /// Forgets the changes Lua made: the user acted, so what changes next is their doing. Without
    /// this, holding a key that moves focus, with a handler that changes something on each focus
    /// change, would look like handlers causing each other.
    pub fn user_input(&mut self) {
        self.changed = None;
        self.carried = None;
    }

    /// Starts a refresh: the changes Lua made before it are carried into the events it fires.
    pub fn begin_refresh(&mut self) {
        self.carried = self.changed.take();
    }

    /// Ends a refresh. The changes Lua made during it carry into the next one.
    pub fn end_refresh(&mut self) {
        self.carried = None;
    }

    /// The depth of `event` fired while no Lua is running: a [`LUA_CAUSED_SIGNALS`] event
    /// continues the depth of a change Lua made, other events start at 0.
    fn event_depth(&self, event: &str) -> u32 {
        if !LUA_CAUSED_SIGNALS.contains(&event) {
            return 0;
        }
        self.carried.max(self.changed).unwrap_or(0)
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

/// The window property signals.
const WINDOW_PROPERTY_SIGNALS: &[&str] = &[
    "window-title-changed",
    "window-fullscreen-changed",
    "window-floating-changed",
    "window-urgent-changed",
    "window-workspace-changed",
];

/// The properties behind the property signals, as Lua last saw them.
///
/// Only kept while something listens to a property signal; see
/// [`State::refresh_lua_properties`].
#[derive(Default)]
pub struct LuaProperties {
    windows: HashMap<u64, WindowSnapshot>,
    workspace_names: HashMap<u64, Option<String>>,
    output_modes: HashMap<String, Option<String>>,
}

impl LuaProperties {
    /// Whether nothing is kept, as when no handler listens to a property signal.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty() && self.workspace_names.is_empty() && self.output_modes.is_empty()
    }
}

/// The property signals that `old` and `new` (the same window) differ in, with the old values.
fn window_property_changes(
    old: &WindowSnapshot,
    new: &WindowSnapshot,
) -> Vec<(&'static str, PropertyValue)> {
    let mut changes = Vec::new();
    if old.title != new.title {
        let title = old
            .title
            .clone()
            .map_or(PropertyValue::Nil, PropertyValue::String);
        changes.push(("window-title-changed", title));
    }
    if old.is_fullscreen != new.is_fullscreen {
        changes.push((
            "window-fullscreen-changed",
            PropertyValue::Bool(old.is_fullscreen),
        ));
    }
    if old.is_floating != new.is_floating {
        changes.push((
            "window-floating-changed",
            PropertyValue::Bool(old.is_floating),
        ));
    }
    if old.is_urgent != new.is_urgent {
        changes.push(("window-urgent-changed", PropertyValue::Bool(old.is_urgent)));
    }
    if (old.workspace_idx, &old.workspace_name) != (new.workspace_idx, &new.workspace_name) {
        let workspace = match old.workspace_idx {
            Some(idx) => PropertyValue::Workspace {
                idx,
                name: old.workspace_name.clone(),
            },
            None => PropertyValue::Nil,
        };
        changes.push(("window-workspace-changed", workspace));
    }
    changes
}

/// Records the `new` values in `map`, and returns the keys whose value changed, with their old
/// value. Keys seen for the first time are only recorded, and keys that are gone are forgotten.
fn diff_values<K: Clone + Eq + std::hash::Hash, V: Clone + PartialEq>(
    map: &mut HashMap<K, V>,
    new: impl IntoIterator<Item = (K, V)>,
) -> Vec<(K, V)> {
    let mut seen = HashSet::new();
    let mut changed = Vec::new();
    for (key, value) in new {
        seen.insert(key.clone());
        if let Some(old) = map.insert(key.clone(), value.clone()) {
            if old != value {
                changed.push((key, old));
            }
        }
    }
    map.retain(|key, _| seen.contains(key));
    changed
}

/// The event loop timers of the Lua timers.
#[derive(Default)]
pub struct LuaTimerSources {
    sources: HashMap<u64, RegistrationToken>,
    /// The timer whose callback is running. Its event loop timer is re-armed or dropped by the
    /// callback's return value, so it must not be touched meanwhile.
    firing: Option<u64>,
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

    fn window_geometry(&mut self, id: u64) -> Option<neori_config::WindowGeometry> {
        let global_space = &self.neori.global_space;
        // A window has a position on screen only if its workspace is the one its monitor shows.
        let shown: HashSet<_> = self
            .neori
            .layout
            .monitors()
            .map(|mon| mon.active_workspace_ref().id())
            .collect();
        let mut geometry = None;
        self.neori
            .layout
            .with_windows(|mapped, output, ws_id, layout| {
                if geometry.is_some() || mapped.id().get() != id {
                    return;
                }
                let output_loc = output
                    .filter(|_| ws_id.is_some_and(|ws| shown.contains(&ws)))
                    .and_then(|o| global_space.output_geometry(o))
                    .map(|g| g.loc.to_f64());
                let position =
                    output_loc
                        .zip(layout.tile_pos_in_workspace_view)
                        .map(|(output, (x, y))| {
                            let (dx, dy) = layout.window_offset_in_tile;
                            (output.x + x + dx, output.y + y + dy)
                        });
                geometry = Some(neori_config::WindowGeometry {
                    position,
                    width: layout.window_size.0,
                    height: layout.window_size.1,
                });
            });
        geometry
    }

    fn window_at(&mut self, x: f64, y: f64) -> Option<WindowSnapshot> {
        let mapped = self
            .neori
            .window_under(smithay::utils::Point::from((x, y)))?;
        Some(snapshot_mapped(&self.neori, mapped))
    }

    fn kill_window(&mut self, id: u64) -> Result<(), String> {
        let (_, mapped) = self
            .neori
            .layout
            .windows()
            .find(|(_, m)| m.id().get() == id)
            .ok_or("this window is gone")?;
        let pid = mapped
            .credentials()
            .map(|c| c.pid)
            .ok_or("this window's process is unknown")?;
        if Some(pid) == xwayland_pid(&self.neori) {
            return Err(
                "can't kill an X11 window: its process, xwayland-satellite, serves every \
                        X11 app; use w:close() instead"
                    .to_owned(),
            );
        }
        kill_process(pid)
    }

    fn set_window_opacity(&mut self, id: u64, opacity: Option<f32>) -> Result<(), String> {
        let window = self
            .neori
            .layout
            .workspaces_mut()
            .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id))
            .ok_or("this window is gone")?;
        window.set_opacity_override(opacity);
        let config = self.neori.config.borrow();
        window.recompute_window_rules(&config.window_rules, self.neori.is_at_startup);
        drop(config);
        // FIXME: granular
        self.neori.queue_redraw_all();
        Ok(())
    }

    fn begin_window_move(&mut self, id: u64) -> Result<(), String> {
        let (window, button, location) = self.prepare_window_grab(id)?;
        self.neori.layout.activate_window(&window);
        let start_data = PointerGrabStartData {
            focus: None,
            button,
            location,
        };
        let icon = CursorIcon::Grabbing;
        let grab = crate::input::move_grab::MoveGrab::new(
            self,
            AnyStartData::Pointer(start_data),
            window,
            false,
            Some(icon),
        )
        .ok_or("the pointer isn't on an output")?;
        let pointer = self.neori.seat.get_pointer().unwrap();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
        self.neori
            .cursor_manager
            .set_cursor_image(CursorImageStatus::Named(icon));
        // FIXME: granular.
        self.neori.queue_redraw_all();
        Ok(())
    }

    fn begin_window_resize(&mut self, id: u64, edges: Option<&str>) -> Result<(), String> {
        let edges = match edges {
            Some(edges) => parse_resize_edges(edges)?,
            None => None,
        };
        let (window, button, location) = self.prepare_window_grab(id)?;
        // Without edges, the ones nearest the pointer if it's over the window, like Mod+RMB.
        let edges = edges
            .or_else(|| {
                let (output, pos) = self.neori.output_under(location)?;
                let under = self.neori.window_under_cursor()?;
                if under.window != window {
                    return None;
                }
                self.neori.layout.resize_edges_under(output, pos)
            })
            .filter(|edges| !edges.is_empty())
            .unwrap_or(ResizeEdge::BOTTOM_RIGHT);

        self.neori.layout.activate_window(&window);
        if !self
            .neori
            .layout
            .interactive_resize_begin(window.clone(), edges)
        {
            return Err("this window can't be resized now".to_owned());
        }
        let start_data = PointerGrabStartData {
            focus: None,
            button,
            location,
        };
        let grab =
            crate::input::resize_grab::ResizeGrab::new(AnyStartData::Pointer(start_data), window);
        let pointer = self.neori.seat.get_pointer().unwrap();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
        self.neori
            .cursor_manager
            .set_cursor_image(CursorImageStatus::Named(edges.cursor_icon()));
        // FIXME: granular.
        self.neori.queue_redraw_all();
        Ok(())
    }

    fn window_parent(&mut self, id: u64) -> Option<WindowSnapshot> {
        let (_, mapped) = self
            .neori
            .layout
            .windows()
            .find(|(_, m)| m.id().get() == id)?;
        let parent = mapped.toplevel().parent()?;
        let (_, parent) = self
            .neori
            .layout
            .windows()
            .find(|(_, m)| m.toplevel().wl_surface() == &parent)?;
        Some(snapshot_mapped(&self.neori, parent))
    }

    fn layers(&mut self) -> Vec<neori_config::LayerSnapshot> {
        layer_snapshots(&self.neori)
    }

    fn close_layer(&mut self, id: u64) -> Result<(), String> {
        let layer = find_layer(&self.neori, id).ok_or("this layer surface is gone")?;
        layer.layer_surface().send_close();
        Ok(())
    }

    fn kill_layer(&mut self, id: u64) -> Result<(), String> {
        let layer = find_layer(&self.neori, id).ok_or("this layer surface is gone")?;
        let pid = crate::utils::get_credentials_for_surface(layer.wl_surface())
            .map(|c| c.pid)
            .ok_or("this layer surface's process is unknown")?;
        kill_process(pid)
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
        self.lua_events.note_change();
        let running = self.lua_events.current;
        let allow_when_locked = running.is_some_and(|r| r.allow_when_locked);
        let time = running.map_or_else(InputTime::now, |r| r.time);
        State::do_action(self, action, allow_when_locked, time);
    }

    fn output_action(&mut self, output: &str, action: neori_ipc::OutputAction) {
        self.lua_events.note_change();
        self.apply_transient_output_config(output, action);
    }

    fn warp(&mut self, x: f64, y: f64) {
        // Moving the pointer can focus another output.
        self.lua_events.note_change();
        self.move_cursor(smithay::utils::Point::from((x, y)));
    }

    fn device_op(&mut self, sysname: &str, op: DeviceOp) {
        apply_device_op(&mut self.neori, sysname, op);
    }

    fn device_mapped_output(&mut self, sysname: &str) -> Option<String> {
        self.neori.device_output_maps.get(sysname).cloned()
    }

    fn set_env(&mut self, name: &str, value: Option<&str>) {
        let mut env = crate::utils::spawning::CHILD_ENV.write().unwrap();
        let value = value.map(str::to_owned);
        match env.0.iter_mut().find(|var| var.name == name) {
            Some(var) => var.value = value,
            None => env.0.push(neori_config::EnvironmentVariable {
                name: name.to_owned(),
                value,
            }),
        }
    }

    fn get_env(&mut self, name: &str) -> Option<String> {
        let env = crate::utils::spawning::CHILD_ENV.read().unwrap();
        match env.0.iter().find(|var| var.name == name) {
            Some(var) => var.value.clone(),
            None => std::env::var(name).ok(),
        }
    }

    fn current_mode(&mut self) -> Option<String> {
        self.neori.active_mode.clone()
    }

    fn arm_timer(&mut self, id: u64, timeout: Duration) {
        self.arm_lua_timer(id, timeout);
    }

    fn disarm_timer(&mut self, id: u64) {
        if self.lua_timers.firing == Some(id) {
            // Firing: fire_lua_timer() drops it after the callback.
            return;
        }
        if let Some(token) = self.lua_timers.sources.remove(&id) {
            self.neori.event_loop.remove(token);
        }
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
        self.run_lua_at(0, label, call, args, allow_when_locked, time);
    }

    /// [`State::run_lua`] for a callback `depth` levels deep, when Lua isn't running.
    fn run_lua_at<A: IntoLuaMulti + Clone + 'static>(
        &mut self,
        depth: u32,
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
                self.report_lua_loop(label, depth);
            }
            return;
        }
        if depth > MAX_LUA_EVENT_DEPTH {
            self.report_lua_loop(label, depth);
            return;
        }
        let running = Running {
            depth,
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

    /// Reports a callback dropped because Lua events caused each other too many levels deep.
    fn report_lua_loop(&mut self, label: &str, depth: u32) {
        warn!(
            "Lua {label} callback dropped: Lua events caused each other {depth} levels deep; do \
             handlers keep triggering each other?"
        );
        self.neori.config_error_notification.show();
    }

    /// Reports a Lua callback error: logged, and shown as the config error notification.
    fn report_lua_result(&mut self, label: &str, result: Result<(), String>) {
        if let Err(err) = result {
            warn!("Lua {label} callback error: {err}");
            self.neori.config_error_notification.show();
        }
    }

    /// Runs Lua code from `neori msg lua` in the config's Lua state, like a callback: reads are
    /// live and changes apply right away. Lua events it causes are delivered after it.
    pub fn eval_lua(
        &mut self,
        code: &str,
    ) -> Result<neori_config::parse_lua::LuaEvalOutput, String> {
        let eval = self.scripting.eval(code)?;
        // Like an IPC action, it runs as if it didn't come from an input.
        let running = Running {
            depth: self.lua_events.next_depth(),
            allow_when_locked: false,
            time: InputTime::now(),
        };
        let result =
            self.with_lua_running(running, |state| with_script_context(state, || eval.run()));
        self.deliver_queued_lua_events();
        result
    }

    /// Switches the active keybinding mode (`None` for the normal binds), and fires
    /// `mode-changed` if it changed.
    pub fn set_bind_mode(&mut self, mode: Option<String>) {
        if self.neori.active_mode == mode {
            return;
        }
        let old = std::mem::replace(&mut self.neori.active_mode, mode.clone());
        self.neori.hotkey_overlay.set_mode(mode.clone());
        self.neori.queue_redraw_all();
        self.run_lua_event("mode-changed", None, |_| (mode, old));
    }

    /// Fires a pointer or gesture signal with a table of fields, built only when a handler
    /// exists. Skipped while the screen is locked, so input meant for the lock screen stays
    /// there.
    pub fn fire_input_event(
        &mut self,
        event: &str,
        fields: impl FnOnce(&mut Self) -> Vec<(&'static str, neori_config::EventValue)>,
    ) {
        if self.neori.is_locked() {
            return;
        }
        self.run_lua_event(event, None, |state| neori_config::EventTable(fields(state)));
    }

    /// Fires `layer-added` or `layer-removed` for a layer-shell surface.
    ///
    /// Call it with no layer map locked: handlers can list the layers.
    pub fn fire_layer_event(&mut self, event: &str, snapshot: neori_config::LayerSnapshot) {
        let target = SignalTarget::Layer(snapshot.id);
        self.run_lua_event(event, Some(target), |_| neori_config::LuaLayer(snapshot));
    }

    /// Fires `device-added` or `device-removed` for a libinput device.
    pub fn fire_device_event(&mut self, event: &str, snapshot: DeviceSnapshot) {
        let target = SignalTarget::Device(snapshot.sysname.clone());
        self.run_lua_event(event, Some(target), |_| LuaDevice(snapshot));
    }

    /// Fires a lifecycle event without arguments: `config-reloaded` or `shutdown`.
    pub fn fire_lua_lifecycle_event(&mut self, event: &str) {
        self.run_lua_event(event, None, |_| ());
    }

    /// Fires the property signals (`window-title-changed`, …) for what changed since the last
    /// refresh.
    ///
    /// Compares the same state that Lua reads, so a handler sees the new value. Costs one lookup
    /// per signal while no handler for a property signal is connected, and keeps no state then;
    /// the first refresh after a handler is connected only records the current values.
    pub fn refresh_lua_properties(&mut self) {
        let _span = tracy_client::span!("State::refresh_lua_properties");

        if WINDOW_PROPERTY_SIGNALS
            .iter()
            .any(|event| self.scripting.has_handlers(event))
        {
            let mut changes = Vec::new();
            let mut seen = HashSet::new();
            for snapshot in window_snapshots(&self.neori) {
                seen.insert(snapshot.id);
                if let Some(old) = self.lua_properties.windows.get(&snapshot.id) {
                    for (event, old_value) in window_property_changes(old, &snapshot) {
                        changes.push((event, snapshot.clone(), old_value));
                    }
                }
                self.lua_properties.windows.insert(snapshot.id, snapshot);
            }
            self.lua_properties
                .windows
                .retain(|id, _| seen.contains(id));
            for (event, snapshot, old_value) in changes {
                let target = SignalTarget::Window(snapshot.id);
                self.run_lua_event(event, Some(target), |_| (LuaWindow(snapshot), old_value));
            }
        } else {
            self.lua_properties.windows.clear();
        }

        if self.scripting.has_handlers("workspace-name-changed") {
            let snapshots = workspace_snapshots(&self.neori);
            let names = snapshots.iter().map(|ws| (ws.id, ws.name.clone()));
            let changed = diff_values(&mut self.lua_properties.workspace_names, names);
            for (id, old_name) in changed {
                let Some(snapshot) = snapshots.iter().find(|ws| ws.id == id).cloned() else {
                    continue;
                };
                let old = old_name.map_or(PropertyValue::Nil, PropertyValue::String);
                let target = SignalTarget::Workspace(id);
                self.run_lua_event("workspace-name-changed", Some(target), |_| {
                    (LuaWorkspace(snapshot), old)
                });
            }
        } else {
            self.lua_properties.workspace_names.clear();
        }

        if self.scripting.has_handlers("output-mode-changed") {
            let snapshots = output_snapshots(self);
            let modes = snapshots.iter().map(|o| (o.name.clone(), o.mode.clone()));
            let changed = diff_values(&mut self.lua_properties.output_modes, modes);
            for (name, old_mode) in changed {
                let Some(snapshot) = snapshots.iter().find(|o| o.name == name).cloned() else {
                    continue;
                };
                let old = old_mode.map_or(PropertyValue::Nil, PropertyValue::String);
                let target = SignalTarget::Output(name);
                self.run_lua_event("output-mode-changed", Some(target), |_| {
                    (LuaOutput(snapshot), old)
                });
            }
        } else {
            self.lua_properties.output_modes.clear();
        }
    }

    /// Installs a freshly loaded Lua host, replacing the current one: the old config's timers are
    /// dropped, and the timers the new config started while loading are armed.
    pub fn install_script_host(&mut self, host: neori_config::ScriptHost) {
        for (_, token) in self.lua_timers.sources.drain() {
            self.neori.event_loop.remove(token);
        }
        self.scripting = host;
        if let Some(sink) = &self.print_log_sink {
            self.scripting.set_print_log(std::sync::Arc::clone(sink));
        }
        for (id, timeout) in self.scripting.running_timers() {
            self.arm_lua_timer(id, timeout);
        }
    }

    /// Arms the event loop timer of Lua timer `id`, restarting it if it's already armed.
    fn arm_lua_timer(&mut self, id: u64, timeout: Duration) {
        if self.lua_timers.firing == Some(id) {
            // Firing: fire_lua_timer() re-arms it after the callback.
            return;
        }
        if let Some(token) = self.lua_timers.sources.remove(&id) {
            self.neori.event_loop.remove(token);
        }
        let token = self
            .neori
            .event_loop
            .insert_source(Timer::from_duration(timeout), move |_, _, state| {
                state.fire_lua_timer(id)
            });
        match token {
            Ok(token) => {
                self.lua_timers.sources.insert(id, token);
            }
            Err(err) => warn!("error arming a Lua timer: {err:?}"),
        }
    }

    /// Runs the callback of Lua timer `id`, and tells the event loop whether to fire it again.
    fn fire_lua_timer(&mut self, id: u64) -> TimeoutAction {
        if let Some(call) = self.scripting.timer_call(id) {
            self.lua_timers.firing = Some(id);
            self.run_lua("timer", call, LuaTimer(id), false, InputTime::now());
            self.lua_timers.firing = None;
        }
        match self.scripting.timer_rearm(id) {
            Some(timeout) => TimeoutAction::ToDuration(timeout),
            None => {
                self.lua_timers.sources.remove(&id);
                TimeoutAction::Drop
            }
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
            let depth = self.lua_events.event_depth(event);
            self.run_lua_at(depth, event, call, args, false, InputTime::now());
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

    let geometry = neori.global_space.output_geometry(output);
    let size = geometry.map(|g| (g.size.w, g.size.h));
    let physical_size = Some((props.size.w, props.size.h)).filter(|&(w, h)| w > 0 && h > 0);
    let workarea = geometry.map(|g| {
        let area = smithay::desktop::layer_map_for_output(output).non_exclusive_zone();
        (
            g.loc.x + area.loc.x,
            g.loc.y + area.loc.y,
            area.size.w,
            area.size.h,
        )
    });

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
        size,
        physical_size,
        workarea,
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
pub(crate) fn device_snapshot(device: &smithay::reexports::input::Device) -> DeviceSnapshot {
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
    // A compositor setting rather than a libinput one, kept by sysname like the config's.
    if let DeviceOp::MapToOutput(output) = op {
        match output {
            Some(output) => neori.device_output_maps.insert(sysname.to_owned(), output),
            None => neori.device_output_maps.remove(sysname),
        };
        return;
    }
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
        // Handled above.
        DeviceOp::MapToOutput(_) => (),
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
        DeviceOp::SetAccelProfile(profile) => {
            let _ = device.config_accel_set_profile(profile.into());
        }
        DeviceOp::SetScrollMethod(method) => {
            let _ = device.config_scroll_set_method(method.into());
        }
        DeviceOp::SetClickMethod(method) => {
            let _ = device.config_click_set_method(method.into());
        }
        DeviceOp::SetDwt(enable) => {
            let _ = device.config_dwt_set_enabled(enable);
        }
        DeviceOp::SetMiddleEmulation(enable) => {
            let _ = device.config_middle_emulation_set_enabled(enable);
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

    #[test]
    fn events_after_a_lua_change_continue_its_depth() {
        let mut queue = LuaEventQueue::default();
        assert_eq!(queue.event_depth("window-focused"), 0);

        // A callback 3 levels deep changes something, during a refresh.
        queue.begin_refresh();
        queue.current = running(3);
        queue.note_change();
        queue.current = None;
        // Events later in the same refresh continue it, but only those Lua can cause.
        assert_eq!(queue.event_depth("window-focused"), 4);
        assert_eq!(queue.event_depth("window-floating-changed"), 4);
        assert_eq!(queue.event_depth("window-title-changed"), 0);
        assert_eq!(queue.event_depth("window-urgent-changed"), 0);
        assert_eq!(queue.event_depth("pointer-button"), 0);
        queue.end_refresh();

        // And so do events in the next refresh.
        queue.begin_refresh();
        assert_eq!(queue.event_depth("window-focused"), 4);
        queue.end_refresh();

        // A refresh without a change ends it.
        queue.begin_refresh();
        assert_eq!(queue.event_depth("window-focused"), 0);
        queue.end_refresh();
    }

    #[test]
    fn user_input_ends_the_depth_of_lua_changes() {
        let mut queue = LuaEventQueue {
            current: running(5),
            ..Default::default()
        };
        queue.note_change();
        queue.current = None;
        queue.begin_refresh();
        queue.current = running(2);
        queue.note_change();
        queue.current = None;
        assert_eq!(queue.event_depth("window-focused"), 6);

        queue.user_input();
        assert_eq!(queue.event_depth("window-focused"), 0);
    }
}
