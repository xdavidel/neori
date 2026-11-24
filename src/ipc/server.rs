use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::{env, io, process};

use anyhow::Context;
use async_channel::{Receiver, Sender, TrySendError};
use calloop::futures::Scheduler;
use calloop::io::Async;
use directories::BaseDirs;
use futures_util::io::{AsyncReadExt, BufReader};
use futures_util::{select_biased, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, FutureExt as _};
use neori_config::OutputName;
use neori_ipc::state::{EventStreamState, EventStreamStatePart as _};
use neori_ipc::{
    Action, Event, KeyboardLayouts, OutputConfigChanged, Overview, Reply, Request, Response,
    Timestamp, WindowLayout, Workspace,
};
use smithay::backend::input::InputTime;
use smithay::desktop::layer_map_for_output;
use smithay::input::pointer::{
    CursorIcon, CursorImageStatus, Focus, GrabStartData as PointerGrabStartData,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::rustix::fs::unlink;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};

use crate::backend::IpcOutputMap;
use crate::handlers::image_copy_capture::{source_mapped_id, source_output};
use crate::input::pick_window_grab::PickWindowGrab;
use crate::layout::workspace::WorkspaceId;
use crate::neori::State;
use crate::utils::{version, with_toplevel_role};
use crate::window::Mapped;

// If an event stream client fails to read events fast enough that we accumulate more than this
// number in our buffer, we drop that event stream client.
const EVENT_STREAM_BUFFER_SIZE: usize = 64;

pub struct IpcServer {
    /// Path to the IPC socket.
    ///
    /// This is `None` when creating `IpcServer` without a socket.
    pub socket_path: Option<PathBuf>,
    event_streams: Rc<RefCell<Vec<EventStreamSender>>>,
    event_stream_state: Rc<RefCell<EventStreamState>>,
}

struct ClientCtx {
    event_loop: LoopHandle<'static, State>,
    scheduler: Scheduler<()>,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    event_streams: Rc<RefCell<Vec<EventStreamSender>>>,
    event_stream_state: Rc<RefCell<EventStreamState>>,
}

struct EventStreamClient {
    events: Receiver<Event>,
    disconnect: Receiver<()>,
    write: Box<dyn AsyncWrite + Unpin>,
}

struct EventStreamSender {
    events: Sender<Event>,
    disconnect: Sender<()>,
}

impl IpcServer {
    pub fn start(
        event_loop: &LoopHandle<'static, State>,
        wayland_socket_name: Option<&OsStr>,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("Ipc::start");

        let mut server = Self {
            socket_path: None,
            event_streams: Rc::new(RefCell::new(Vec::new())),
            event_stream_state: Rc::new(RefCell::new(EventStreamState::default())),
        };
        server.start_socket(event_loop, wayland_socket_name)?;
        Ok(server)
    }

    /// Starts listening on a socket, keeping the existing event stream state.
    ///
    /// Does nothing when `wayland_socket_name` is `None`.
    pub fn start_socket(
        &mut self,
        event_loop: &LoopHandle<'static, State>,
        wayland_socket_name: Option<&OsStr>,
    ) -> anyhow::Result<()> {
        let Some(wayland_socket_name) = wayland_socket_name else {
            return Ok(());
        };

        let wayland_socket_name = wayland_socket_name.to_string_lossy();
        let socket_name = format!("neori.{wayland_socket_name}.{}.sock", process::id());
        let mut socket_path = socket_dir();
        socket_path.push(socket_name);

        let listener = UnixListener::bind(&socket_path).context("error binding socket")?;
        listener
            .set_nonblocking(true)
            .context("error setting socket to non-blocking")?;

        let source = Generic::new(listener, Interest::READ, Mode::Level);
        event_loop
            .insert_source(source, |_, socket, state| {
                match socket.accept() {
                    Ok((stream, _)) => on_new_ipc_client(state, stream),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                    Err(e) => return Err(e),
                }

                Ok(PostAction::Continue)
            })
            .unwrap();

        self.socket_path = Some(socket_path);
        Ok(())
    }

    fn send_event(&self, event: Event) {
        let mut streams = self.event_streams.borrow_mut();
        let mut to_remove = Vec::new();
        for (idx, stream) in streams.iter_mut().enumerate() {
            match stream.events.try_send(event.clone()) {
                Ok(()) => (),
                Err(TrySendError::Closed(_)) => to_remove.push(idx),
                Err(TrySendError::Full(_)) => {
                    warn!(
                        "disconnecting IPC event stream client \
                         because it is reading events too slowly"
                    );
                    to_remove.push(idx);
                }
            }
        }

        for idx in to_remove.into_iter().rev() {
            let stream = streams.swap_remove(idx);
            let _ = stream.disconnect.send_blocking(());
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(socket_path) = &self.socket_path {
            let _ = unlink(socket_path);
        }
    }
}

fn socket_dir() -> PathBuf {
    BaseDirs::new()
        .as_ref()
        .and_then(|x| x.runtime_dir())
        .map(|x| x.to_owned())
        .unwrap_or_else(env::temp_dir)
}

fn on_new_ipc_client(state: &mut State, stream: UnixStream) {
    let _span = tracy_client::span!("on_new_ipc_client");
    trace!("new IPC client connected");

    let stream = match state.neori.event_loop.adapt_io(stream) {
        Ok(stream) => stream,
        Err(err) => {
            warn!("error making IPC stream async: {err:?}");
            return;
        }
    };

    let ipc_server = state.neori.ipc_server.as_ref().unwrap();

    let ctx = ClientCtx {
        event_loop: state.neori.event_loop.clone(),
        scheduler: state.neori.scheduler.clone(),
        ipc_outputs: state.backend.ipc_outputs(),
        event_streams: ipc_server.event_streams.clone(),
        event_stream_state: ipc_server.event_stream_state.clone(),
    };

    let future = async move {
        if let Err(err) = handle_client(ctx, stream).await {
            warn!("error handling IPC client: {err:?}");
        }
    };
    if let Err(err) = state.neori.scheduler.schedule(future) {
        warn!("error scheduling IPC stream future: {err:?}");
    }
}

async fn handle_client(ctx: ClientCtx, stream: Async<'static, UnixStream>) -> anyhow::Result<()> {
    let (read, mut write) = stream.split();
    let mut read = BufReader::new(read);

    loop {
        // Don't keep buf around to avoid clients wasting RAM by filling it with bogus data.
        let mut buf = Vec::new();
        let res = read.read_until(b'\n', &mut buf).await;
        match res {
            Ok(0) => return Ok(()),
            Ok(_) => (),
            // Normal client disconnection.
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            Err(err) => {
                return Err(err).context("error reading request");
            }
        }

        let request = serde_json::from_slice(&buf)
            .context("error parsing request")
            .map_err(|err| err.to_string());
        let requested_error = matches!(request, Ok(Request::ReturnError));
        let requested_event_stream = matches!(request, Ok(Request::EventStream));

        let reply = match request {
            Ok(request) => process(&ctx, request).await,
            Err(err) => Err(err),
        };

        if let Err(err) = &reply {
            if !requested_error {
                warn!("error processing IPC request: {err:?}");
            }
        }

        buf.clear();
        serde_json::to_writer(&mut buf, &reply).context("error formatting reply")?;
        buf.push(b'\n');
        write.write_all(&buf).await.context("error writing reply")?;

        if requested_event_stream {
            let (events_tx, events_rx) = async_channel::bounded(EVENT_STREAM_BUFFER_SIZE);
            let (disconnect_tx, disconnect_rx) = async_channel::bounded(1);

            // Spawn a task for the client.
            let client = EventStreamClient {
                events: events_rx,
                disconnect: disconnect_rx,
                write: Box::new(write) as _,
            };
            let future = async move {
                if let Err(err) = handle_event_stream_client(client).await {
                    warn!("error handling IPC event stream client: {err:?}");
                }
            };
            if let Err(err) = ctx.scheduler.schedule(future) {
                warn!("error scheduling IPC event stream future: {err:?}");
            }

            // Send the initial state.
            {
                let state = ctx.event_stream_state.borrow();
                for event in state.replicate() {
                    events_tx
                        .try_send(event)
                        .expect("initial event burst had more events than buffer size");
                }
            }

            // Add it to the list.
            {
                let mut streams = ctx.event_streams.borrow_mut();
                let sender = EventStreamSender {
                    events: events_tx,
                    disconnect: disconnect_tx,
                };
                streams.push(sender);
            }

            return Ok(());
        }
    }
}

async fn process(ctx: &ClientCtx, request: Request) -> Reply {
    let response = match request {
        Request::ReturnError => return Err(String::from("example compositor error")),
        Request::Version => Response::Version(version()),
        Request::Outputs => {
            let ipc_outputs = ctx.ipc_outputs.lock().unwrap().clone();
            let outputs = ipc_outputs.values().cloned().map(|o| (o.name.clone(), o));
            Response::Outputs(outputs.collect())
        }
        Request::Workspaces => {
            let state = ctx.event_stream_state.borrow();
            let workspaces = state.workspaces.workspaces.values().cloned().collect();
            Response::Workspaces(workspaces)
        }
        Request::Windows => {
            let state = ctx.event_stream_state.borrow();
            let windows = state.windows.windows.values().cloned().collect();
            Response::Windows(windows)
        }
        Request::Layers => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                let mut layers = Vec::new();
                for output in state.neori.global_space.outputs() {
                    let name = output.name();
                    for surface in layer_map_for_output(output).layers() {
                        let layer = match surface.layer() {
                            Layer::Background => neori_ipc::Layer::Background,
                            Layer::Bottom => neori_ipc::Layer::Bottom,
                            Layer::Top => neori_ipc::Layer::Top,
                            Layer::Overlay => neori_ipc::Layer::Overlay,
                        };
                        let keyboard_interactivity =
                            match surface.cached_state().keyboard_interactivity {
                                KeyboardInteractivity::None => {
                                    neori_ipc::LayerSurfaceKeyboardInteractivity::None
                                }
                                KeyboardInteractivity::Exclusive => {
                                    neori_ipc::LayerSurfaceKeyboardInteractivity::Exclusive
                                }
                                KeyboardInteractivity::OnDemand => {
                                    neori_ipc::LayerSurfaceKeyboardInteractivity::OnDemand
                                }
                            };

                        layers.push(neori_ipc::LayerSurface {
                            namespace: surface.namespace().to_owned(),
                            output: name.clone(),
                            layer,
                            keyboard_interactivity,
                        });
                    }
                }

                let _ = tx.send_blocking(layers);
            });
            let result = rx.recv().await;
            let layers = result.map_err(|_| String::from("error getting layers info"))?;
            Response::Layers(layers)
        }
        Request::KeyboardLayouts => {
            let state = ctx.event_stream_state.borrow();
            let layout = state.keyboard_layouts.keyboard_layouts.clone();
            let layout = layout.expect("keyboard layouts should be set at startup");
            Response::KeyboardLayouts(layout)
        }
        Request::FocusedWindow => {
            let state = ctx.event_stream_state.borrow();
            let windows = &state.windows.windows;
            let window = windows.values().find(|win| win.is_focused).cloned();
            Response::FocusedWindow(window)
        }
        Request::PickWindow => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                let pointer = state.neori.seat.get_pointer().unwrap();
                let start_data = PointerGrabStartData {
                    focus: None,
                    button: 0,
                    location: pointer.current_location(),
                };
                let grab = PickWindowGrab::new(start_data);
                // The `WindowPickGrab` ungrab handler will cancel the previous ongoing pick, if
                // any.
                pointer.set_grab(state, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
                state.neori.pick_window = Some(tx);
                state
                    .neori
                    .cursor_manager
                    .set_cursor_image(CursorImageStatus::Named(CursorIcon::Crosshair));
                // Redraw to update the cursor.
                state.neori.queue_redraw_all();
            });
            let result = rx.recv().await;
            let id = result.map_err(|_| String::from("error getting picked window info"))?;
            let window = id.and_then(|id| {
                let state = ctx.event_stream_state.borrow();
                state.windows.windows.get(&id.get()).cloned()
            });
            Response::PickedWindow(window)
        }
        Request::PickColor => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                state.handle_pick_color(tx);
            });
            let result = rx.recv().await;
            let color = result.map_err(|_| String::from("error getting picked color"))?;
            Response::PickedColor(color)
        }
        Request::Action(action) => {
            validate_action(&action)?;

            let (tx, rx) = async_channel::bounded(1);

            ctx.event_loop.insert_idle(move |state| {
                // Make sure some logic like workspace clean-up has a chance to run before doing
                // actions.
                state.neori.advance_animations();
                // This action didn't come from an input, so just use the current time.
                let time = InputTime::now();
                state.do_action(action, false, time);
                let _ = tx.send_blocking(());
            });

            // Wait until the action has been processed before returning. This is important for a
            // few actions, for instance for DoScreenTransition this wait ensures that the screen
            // contents were sampled into the texture.
            let _ = rx.recv().await;
            Response::Handled
        }
        Request::Output { output, action } => {
            action.validate()?;

            let ipc_outputs = ctx.ipc_outputs.lock().unwrap();
            let found = ipc_outputs
                .values()
                .any(|o| OutputName::from_ipc_output(o).matches(&output));
            let response = if found {
                OutputConfigChanged::Applied
            } else {
                OutputConfigChanged::OutputWasMissing
            };
            drop(ipc_outputs);

            ctx.event_loop.insert_idle(move |state| {
                state.apply_transient_output_config(&output, action);
            });

            Response::OutputConfigChanged(response)
        }
        Request::CreateVirtualOutput {
            width,
            height,
            refresh_rate,
            name,
        } => {
            let width = width.unwrap_or(1920);
            let height = height.unwrap_or(1080);
            let refresh_rate = refresh_rate.unwrap_or(60);

            let (tx, rx) = async_channel::bounded(1);

            ctx.event_loop.insert_idle(move |state| {
                let result = state.backend.create_virtual_output(
                    &mut state.neori,
                    width,
                    height,
                    refresh_rate,
                    name,
                );
                let _ = tx.send_blocking(result);
            });

            let result = rx
                .recv()
                .await
                .map_err(|_| String::from("error creating virtual output"))?;
            match result {
                Ok(name) => Response::VirtualOutputCreated(name),
                Err(e) => return Err(e),
            }
        }
        Request::RemoveVirtualOutput { name } => {
            let (tx, rx) = async_channel::bounded(1);

            ctx.event_loop.insert_idle(move |state| {
                let result = state.backend.remove_virtual_output(&mut state.neori, &name);
                let _ = tx.send_blocking(result);
            });

            let result = rx
                .recv()
                .await
                .map_err(|_| String::from("error removing virtual output"))?;
            result?;
            Response::Handled
        }
        Request::FocusedOutput => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                let active_output = state
                    .neori
                    .layout
                    .active_output()
                    .map(|output| output.name());

                let output = active_output.and_then(|active_output| {
                    state
                        .backend
                        .ipc_outputs()
                        .lock()
                        .unwrap()
                        .values()
                        .find(|o| o.name == active_output)
                        .cloned()
                });

                let _ = tx.send_blocking(output);
            });
            let result = rx.recv().await;
            let output = result.map_err(|_| String::from("error getting active output info"))?;
            Response::FocusedOutput(output)
        }
        Request::EventStream => Response::Handled,
        Request::OverviewState => {
            let state = ctx.event_stream_state.borrow();
            let is_open = state.overview.is_open;
            Response::OverviewState(Overview { is_open })
        }
        Request::Lua { code } => {
            let (tx, rx) = async_channel::bounded(1);
            ctx.event_loop.insert_idle(move |state| {
                let result = state.eval_lua(&code);
                let _ = tx.send_blocking(result);
            });
            let result = rx
                .recv()
                .await
                .map_err(|_| String::from("error running Lua code"))??;
            Response::LuaEvaluated(neori_ipc::LuaEvaluated {
                printed: result.printed,
                values: result.values,
            })
        }
        Request::Casts => {
            let state = ctx.event_stream_state.borrow();
            let casts = state.casts.casts.values().cloned().collect();
            Response::Casts(casts)
        }
    };

    Ok(response)
}

fn validate_action(action: &Action) -> Result<(), String> {
    if let Action::Screenshot { path, .. }
    | Action::ScreenshotScreen { path, .. }
    | Action::ScreenshotWindow { path, .. }
    | Action::LoadConfigFile { path } = action
    {
        if let Some(path) = path {
            // Relative paths are resolved against the neori compositor's working directory, which
            // is almost certainly not what you want.
            if !Path::new(path).is_absolute() {
                return Err(format!("path must be absolute: {path}"));
            }
        }
    }

    if let Action::LoadConfigFile { path: Some(path) } = action {
        let p = Path::new(path);
        if !p.is_file() {
            return Err(format!("path does not point to a file: {path}"));
        }
    }

    if let Action::SetShakeToFind { settings } = action {
        if let Some(v) = settings.size_multiplier {
            if !(v.is_finite() && v > 0.) {
                return Err(format!(
                    "size_multiplier must be a positive number, got {v}"
                ));
            }
        }
        if settings.window_ms == Some(0) {
            return Err(String::from("window_ms must be greater than zero"));
        }
        if settings.min_reversals == Some(0) {
            return Err(String::from("min_reversals must be greater than zero"));
        }
        if let Some(v) = settings.min_step {
            if !(v.is_finite() && v >= 0.) {
                return Err(format!("min_step must be a non-negative number, got {v}"));
            }
        }
        if let Some(v) = settings.reversal_angle_deg {
            if !(v.is_finite() && v > 0. && v < 180.) {
                return Err(format!(
                    "reversal_angle_deg must be between 0 and 180, got {v}"
                ));
            }
        }
    }

    Ok(())
}

async fn handle_event_stream_client(client: EventStreamClient) -> anyhow::Result<()> {
    let EventStreamClient {
        events,
        disconnect,
        mut write,
    } = client;

    while let Ok(event) = events.recv().await {
        let mut buf = serde_json::to_vec(&event).context("error formatting event")?;
        buf.push(b'\n');

        let res = select_biased! {
            _ = disconnect.recv().fuse() => return Ok(()),
            res = write.write_all(&buf).fuse() => res,
        };

        match res {
            Ok(()) => (),
            // Normal client disconnection.
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            res @ Err(_) => res.context("error writing event")?,
        }
    }

    Ok(())
}

fn make_ipc_window(
    mapped: &Mapped,
    workspace_id: Option<WorkspaceId>,
    layout: WindowLayout,
    is_xwayland: bool,
) -> neori_ipc::Window {
    // Must be computed outside of `with_toplevel_role()`, which holds the surface data lock;
    // `is_fullscreen()` reads the toplevel's committed state and would deadlock otherwise.
    let is_fullscreen = mapped.is_fullscreen();

    with_toplevel_role(mapped.toplevel(), |role| neori_ipc::Window {
        id: mapped.id().get(),
        title: role.title.clone(),
        app_id: role.app_id.clone(),
        pid: mapped.credentials().map(|c| c.pid),
        workspace_id: workspace_id.map(|id| id.get()),
        is_focused: mapped.is_focused(),
        is_floating: mapped.is_floating(),
        is_fullscreen,
        is_window_cast_target: mapped.is_window_cast_target(),
        is_active_in_column: mapped.is_active_in_column(),
        is_xwayland,
        is_urgent: mapped.is_urgent(),
        layout,
        focus_timestamp: mapped.get_focus_timestamp().map(Timestamp::from),
    })
}

impl State {
    pub fn ipc_keyboard_layouts_changed(&mut self) {
        let keyboard = self.neori.seat.get_keyboard().unwrap();
        let keyboard_layouts = keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().unwrap();
            let layouts = xkb.layouts();
            KeyboardLayouts {
                names: layouts
                    .map(|layout| xkb.layout_name(layout).to_owned())
                    .collect(),
                current_idx: xkb.active_layout().0 as u8,
            }
        });

        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let mut state = server.event_stream_state.borrow_mut();
        let state = &mut state.keyboard_layouts;

        let event = Event::KeyboardLayoutsChanged { keyboard_layouts };
        state.apply(event.clone());
        server.send_event(event);
    }

    pub fn ipc_refresh_keyboard_layout_index(&mut self) {
        let keyboard = self.neori.seat.get_keyboard().unwrap();
        let (idx, name) = keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().unwrap();
            let active = xkb.active_layout();
            (active.0 as u8, xkb.layout_name(active).to_owned())
        });

        // Did the layout change since last call? In the happy path IPC's own
        // state.keyboard_layouts.current_idx is the source of truth. On the
        // degraded IPC-absent path (IpcServer::start failed) we fall back to
        // the Neori-side mirror so Lua `layout-changed` still fires.
        let changed = match &self.neori.ipc_server {
            Some(server) => {
                let mut state = server.event_stream_state.borrow_mut();
                let state = &mut state.keyboard_layouts;

                if state.keyboard_layouts.as_ref().unwrap().current_idx == idx {
                    false
                } else {
                    let event = Event::KeyboardLayoutSwitched { idx };
                    state.apply(event.clone());
                    server.send_event(event);
                    true
                }
            }
            None => {
                if self.neori.last_keyboard_layout_idx_fallback == Some(idx) {
                    false
                } else {
                    self.neori.last_keyboard_layout_idx_fallback = Some(idx);
                    true
                }
            }
        };

        // Fire Lua `layout-changed` on the same diff. idx is a bijection with
        // (layout slot, config), so idx unchanged implies name unchanged — no
        // separate name diff needed.
        if changed {
            self.fire_layout_changed_event(name);
        }
    }

    pub fn ipc_refresh_layout(&mut self) {
        self.ipc_refresh_workspaces();
        self.ipc_refresh_windows();
        self.ipc_refresh_overview();
    }

    fn ipc_refresh_workspaces(&mut self) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let _span = tracy_client::span!("State::ipc_refresh_workspaces");

        // Walk layout once; produce IPC events AND the Lua delta payload in the same
        // pass. The IPC refresh runs per tick from State::refresh, so having Lua
        // piggy-back on it means one diff + one walk per tick regardless of whether
        // IPC clients or Lua handlers are consuming — no duplicated work.
        let mut lua_added: Vec<neori_config::WorkspaceSnapshot> = Vec::new();
        let mut lua_removed: Vec<neori_config::WorkspaceSnapshot> = Vec::new();
        let mut lua_focused_changed: Option<neori_config::WorkspaceSnapshot> = None;
        let new_focused_output_name = self.neori.layout.active_output().map(|o| o.name());

        let output_focus_changed = {
            let mut state = server.event_stream_state.borrow_mut();
            let state = &mut state.workspaces;

            // Derive "last known" focused output from IPC state (the workspace with
            // is_focused=true carries the output name). IPC state is also our Lua
            // last-known state, so we don't need a separate field on Neori.
            let old_focused_output_name = state
                .workspaces
                .values()
                .find(|w| w.is_focused)
                .and_then(|w| w.output.clone());
            let output_focus_changed = new_focused_output_name != old_focused_output_name;

            let mut events = Vec::new();
            let layout = &self.neori.layout;
            let focused_ws_id = layout.active_workspace().map(|ws| ws.id().get());

            // Check for workspace changes. Walk every workspace (no early break) so we
            // see every add candidate for Lua even if an earlier structural change
            // already marked need_workspaces_changed. `global_idx` matches the 1-based
            // position-in-walk that Lua `ws:idx()` has always returned.
            //
            // Build the full IPC `Workspace` struct for every workspace in the same
            // pass. If need_workspaces_changed ends true, the fallback event just
            // wraps this Vec — no second walk of layout.workspaces() needed.
            let mut seen = HashSet::new();
            let mut need_workspaces_changed = false;
            let mut global_idx: u16 = 0;
            let mut current_workspaces: Vec<Workspace> = Vec::new();
            for (mon, ws_idx, ws) in layout.workspaces() {
                global_idx = global_idx.saturating_add(1);
                let id = ws.id().get();
                seen.insert(id);

                let is_focused_now = Some(id) == focused_ws_id;

                let new_ws = Workspace {
                    id,
                    idx: u8::try_from(ws.index()).unwrap_or(u8::MAX),
                    name: ws.name().cloned(),
                    output: mon.map(|mon| mon.output_name().clone()),
                    is_urgent: ws.is_urgent(),
                    is_active: mon.is_some_and(|mon| mon.active_workspace_idx() == ws_idx),
                    is_focused: is_focused_now,
                    active_window_id: ws.active_window().map(|win| win.id().get()),
                };

                let Some(ipc_ws) = state.workspaces.get(&id) else {
                    // A new workspace was added.
                    need_workspaces_changed = true;
                    let snap =
                        crate::script::snapshot_workspace(mon, global_idx, ws, focused_ws_id);
                    if is_focused_now {
                        lua_focused_changed = Some(snap.clone());
                    }
                    lua_added.push(snap);
                    current_workspaces.push(new_ws);
                    continue;
                };

                // Check for any changes that we can't signal as individual events.
                if ipc_ws.idx != new_ws.idx
                    || ipc_ws.name != new_ws.name
                    || ipc_ws.output != new_ws.output
                {
                    need_workspaces_changed = true;
                }

                if ipc_ws.active_window_id != new_ws.active_window_id {
                    events.push(Event::WorkspaceActiveWindowChanged {
                        workspace_id: id,
                        active_window_id: new_ws.active_window_id,
                    });
                }

                if new_ws.is_urgent != ipc_ws.is_urgent {
                    events.push(Event::WorkspaceUrgencyChanged {
                        id,
                        urgent: new_ws.is_urgent,
                    });
                }

                // Check if this workspace became focused.
                if is_focused_now && !ipc_ws.is_focused {
                    events.push(Event::WorkspaceActivated { id, focused: true });
                    lua_focused_changed = Some(crate::script::snapshot_workspace(
                        mon,
                        global_idx,
                        ws,
                        focused_ws_id,
                    ));
                    current_workspaces.push(new_ws);
                    continue;
                }

                // Check if this workspace became active (on its monitor).
                if new_ws.is_active && !ipc_ws.is_active {
                    events.push(Event::WorkspaceActivated { id, focused: false });
                }

                current_workspaces.push(new_ws);
            }

            // Check if any workspaces were removed. Capture the last-known snapshot for
            // Lua so `workspace-removed` handlers still see the vanished workspace's
            // metadata.
            for (id, ipc_ws) in state.workspaces.iter() {
                if !seen.contains(id) {
                    need_workspaces_changed = true;
                    lua_removed.push(neori_config::WorkspaceSnapshot {
                        idx: ipc_ws.idx,
                        name: ipc_ws.name.clone(),
                        output_name: ipc_ws.output.clone(),
                        is_focused: ipc_ws.is_focused,
                        id: *id,
                        window_ids: Vec::new(),
                    });
                }
            }

            if need_workspaces_changed {
                events.clear();
                events.push(Event::WorkspacesChanged {
                    workspaces: current_workspaces,
                });
            }

            for event in events {
                state.apply(event.clone());
                server.send_event(event);
            }

            output_focus_changed
        };

        // IPC borrows dropped — now fire Lua events.
        for snap in lua_added {
            self.fire_workspace_event("workspace-added", snap);
        }
        for snap in lua_removed {
            self.neori
                .lua_removed
                .workspaces
                .insert(snap.id, snap.clone());
            self.fire_workspace_event("workspace-removed", snap);
        }
        if let Some(snap) = lua_focused_changed {
            self.fire_workspace_event("workspace-focused", snap);
        }
        if output_focus_changed {
            if let Some(name) = new_focused_output_name {
                self.fire_output_focused_event(name);
            }
        }
    }

    fn ipc_refresh_windows(&mut self) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let _span = tracy_client::span!("State::ipc_refresh_windows");

        let mut state = server.event_stream_state.borrow_mut();
        let state = &mut state.windows;

        let mut events = Vec::new();
        let layout = &self.neori.layout;

        // PID of the xwayland-satellite process, if it is running. X11 windows belong to this
        // process as a Wayland client.
        let xwayland_pid = self.neori.satellite.as_ref().and_then(|s| s.pid());

        let mut batch_change_layouts: Vec<(u64, WindowLayout)> = Vec::new();

        // Check for window changes.
        let mut seen = HashSet::new();
        let mut focused_id = None;
        layout.with_windows(|mapped, _, ws_id, window_layout| {
            let id = mapped.id().get();
            seen.insert(id);

            if mapped.is_focused() {
                focused_id = Some(id);
            }

            let is_xwayland =
                xwayland_pid.is_some_and(|pid| mapped.credentials().is_some_and(|c| c.pid == pid));

            let Some(ipc_win) = state.windows.get(&id) else {
                let window = make_ipc_window(mapped, ws_id, window_layout, is_xwayland);
                events.push(Event::WindowOpenedOrChanged { window });
                return;
            };

            let workspace_id = ws_id.map(|id| id.get());
            let mut changed = ipc_win.workspace_id != workspace_id
                || ipc_win.is_floating != mapped.is_floating()
                || ipc_win.is_fullscreen != mapped.is_fullscreen()
                || ipc_win.is_window_cast_target != mapped.is_window_cast_target()
                || ipc_win.is_active_in_column != mapped.is_active_in_column()
                || ipc_win.is_xwayland != is_xwayland;

            changed |= with_toplevel_role(mapped.toplevel(), |role| {
                ipc_win.title != role.title || ipc_win.app_id != role.app_id
            });

            if changed {
                let window = make_ipc_window(mapped, ws_id, window_layout, is_xwayland);
                events.push(Event::WindowOpenedOrChanged { window });
                return;
            }

            if ipc_win.layout != window_layout {
                batch_change_layouts.push((id, window_layout));
            }

            if mapped.is_focused() && !ipc_win.is_focused {
                events.push(Event::WindowFocusChanged { id: Some(id) });
            }

            let focus_timestamp = mapped.get_focus_timestamp().map(Timestamp::from);
            if focus_timestamp != ipc_win.focus_timestamp {
                events.push(Event::WindowFocusTimestampChanged {
                    id,
                    focus_timestamp,
                });
            }

            let urgent = mapped.is_urgent();
            if urgent != ipc_win.is_urgent {
                events.push(Event::WindowUrgencyChanged { id, urgent })
            }
        });

        // It might make sense to push layout changes after closed windows (since windows about to
        // be closed will occupy the same column/tile positions as the window that moved into this
        // vacated space), but also we are already pushing some layout changes in
        // WindowOpenedOrChanged above, meaning that the receiving end has to handle this case
        // anyway.
        if !batch_change_layouts.is_empty() {
            events.push(Event::WindowLayoutsChanged {
                changes: batch_change_layouts,
            });
        }

        // Check for closed windows.
        let mut ipc_focused_id = None;
        for (id, ipc_win) in &state.windows {
            if !seen.contains(id) {
                events.push(Event::WindowClosed { id: *id });
            }

            if ipc_win.is_focused {
                ipc_focused_id = Some(id);
            }
        }

        // Extra check for focus becoming None, since the checks above only work for focus becoming
        // a different window.
        if focused_id.is_none() && ipc_focused_id.is_some() {
            events.push(Event::WindowFocusChanged { id: None });
        }

        for event in events {
            state.apply(event.clone());
            server.send_event(event);
        }
    }

    pub fn ipc_refresh_overview(&mut self) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let mut state = server.event_stream_state.borrow_mut();
        let state = &mut state.overview;
        let is_open = self.neori.layout.is_overview_open();

        if state.is_open == is_open {
            return;
        }

        let event = Event::OverviewOpenedOrClosed { is_open };
        state.apply(event.clone());
        server.send_event(event);
    }

    pub fn ipc_refresh_casts(&mut self) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let _span = tracy_client::span!("State::ipc_refresh_casts");

        let mut state = server.event_stream_state.borrow_mut();
        let state = &mut state.casts;

        let mut events = Vec::new();
        let mut seen = HashSet::new();

        // Check PipeWire screencasts.
        #[cfg(feature = "xdp-gnome-screencast")]
        {
            // Check pending dynamic casts.
            for pending in &self.neori.casting.pending_dynamic_casts {
                let stream_id = pending.stream_id.get();
                seen.insert(stream_id);

                // Pending dynamic casts don't change any properties, so we only need to check if
                // it's missing from the state.
                if !state.casts.contains_key(&stream_id) {
                    let cast = neori_ipc::Cast {
                        session_id: pending.session_id.get(),
                        stream_id,
                        kind: neori_ipc::CastKind::PipeWire,
                        target: neori_ipc::CastTarget::Nothing {},
                        is_dynamic_target: true,
                        is_active: false,
                        pid: None,
                        pw_node_id: None,
                    };
                    events.push(Event::CastStartedOrChanged { cast });
                }
            }

            // Check active casts.
            for cast in &self.neori.casting.casts {
                let stream_id = cast.stream_id.get();
                seen.insert(stream_id);

                let pw_node_id = cast.node_id();
                if state.casts.get(&stream_id).is_none_or(|existing| {
                    // Only these properties can change.
                    existing.is_active != cast.is_active()
                        || !cast.target.matches(&existing.target)
                        || existing.pw_node_id != pw_node_id
                }) {
                    let cast = neori_ipc::Cast {
                        session_id: cast.session_id.get(),
                        stream_id,
                        kind: neori_ipc::CastKind::PipeWire,
                        target: cast.target.make_ipc(),
                        is_dynamic_target: cast.dynamic_target,
                        is_active: cast.is_active(),
                        pid: None,
                        pw_node_id,
                    };
                    events.push(Event::CastStartedOrChanged { cast });
                }
            }
        }

        // Check screencopy casts.
        //
        // First, clear expired casts. Ideally we'd have a deadline timer, but our 1 second frame
        // callback timer calls refresh regularly, so that's fine as is.
        self.neori.screencopy_state.clear_expired_casts();

        for queue in self.neori.screencopy_state.queues() {
            if let Some(cast_info) = queue.cast() {
                let stream_id = cast_info.stream_id.get();
                seen.insert(stream_id);

                if state.casts.get(&stream_id).is_none_or(|existing| {
                    // Only this property can change.
                    match &existing.target {
                        neori_ipc::CastTarget::Output { name } => *name != cast_info.output_name,
                        _ => true,
                    }
                }) {
                    let cast = neori_ipc::Cast {
                        session_id: cast_info.session_id.get(),
                        stream_id,
                        kind: neori_ipc::CastKind::WlrScreencopy,
                        target: neori_ipc::CastTarget::Output {
                            name: cast_info.output_name.clone(),
                        },
                        is_dynamic_target: false,
                        is_active: true,
                        pid: queue.credentials().map(|creds| creds.pid),
                        pw_node_id: None,
                    };
                    events.push(Event::CastStartedOrChanged { cast });
                }
            }
        }

        // Check ext-image-copy-capture casts.
        //
        // Unlike wlr-screencopy, a session is an explicit protocol object, so
        // the cast lifetime is tied to it. Dead sessions are already dropped by
        // refresh_image_copy_capture(), which runs before this. Sessions which
        // have requested only one frame are screenshots, not casts.
        let output_sessions = self
            .neori
            .image_copy_sessions
            .iter()
            .filter(|s| s.is_cast())
            .map(|s| {
                (
                    neori_ipc::CastKind::ExtImageCopyCapture,
                    s.session_id,
                    s.stream_id,
                    s.session.source(),
                    s.credentials,
                )
            });
        let cursor_sessions = self
            .neori
            .image_copy_cursor_sessions
            .iter()
            .filter(|s| s.is_cast())
            .map(|s| {
                (
                    neori_ipc::CastKind::ExtImageCopyCaptureCursor,
                    s.session_id,
                    s.stream_id,
                    s.session.source(),
                    s.credentials,
                )
            });
        for (kind, session_id, stream_id, source, credentials) in
            output_sessions.chain(cursor_sessions)
        {
            let target = if let Some(output) = source_output(&source) {
                neori_ipc::CastTarget::Output {
                    name: output.name(),
                }
            } else if let Some(id) = source_mapped_id(&source) {
                neori_ipc::CastTarget::Window { id: id.get() }
            } else {
                neori_ipc::CastTarget::Nothing {}
            };

            let stream_id = stream_id.get();
            seen.insert(stream_id);

            if !state.casts.contains_key(&stream_id) {
                let cast = neori_ipc::Cast {
                    session_id: session_id.get(),
                    stream_id,
                    kind,
                    target,
                    is_dynamic_target: false,
                    is_active: true,
                    pid: credentials.map(|creds| creds.pid),
                    pw_node_id: None,
                };
                events.push(Event::CastStartedOrChanged { cast });
            }
        }

        // Check for stopped casts.
        for stream_id in state.casts.keys() {
            if !seen.contains(stream_id) {
                events.push(Event::CastStopped {
                    stream_id: *stream_id,
                });
            }
        }

        for event in events {
            state.apply(event.clone());
            server.send_event(event);
        }
    }

    pub fn ipc_refresh_touchpad(&mut self) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };

        let mut state = server.event_stream_state.borrow_mut();
        let state = &mut state.touchpad;
        let on = !self.neori.touchpad_disabled_by_toggle;

        if state.on == on {
            return;
        }

        let event = Event::TouchpadToggled { on };
        state.apply(event.clone());
        server.send_event(event);
    }

    pub fn ipc_config_loaded(&mut self, failed: bool) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };
        let mut state = server.event_stream_state.borrow_mut();

        let event = Event::ConfigLoaded { failed };
        state.apply(event.clone());
        server.send_event(event);
    }

    pub fn ipc_screenshot_taken(&mut self, path: Option<String>) {
        let Some(server) = &self.neori.ipc_server else {
            return;
        };
        let mut state = server.event_stream_state.borrow_mut();

        let event = Event::ScreenshotCaptured { path };
        state.apply(event.clone());
        server.send_event(event);
    }
}
