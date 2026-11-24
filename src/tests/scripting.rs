//! Tests for the Lua scripting runtime API: function callbacks on binds, `neori.action`,
//! and error containment.

use neori_config::binds::Bind;
use neori_config::{Action, Config};
use smithay::backend::input::{InputTime, KeyState};
use smithay::input::keyboard::{Keysym, ModifiersState};
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;

use super::client::ClientId;
use super::*;

/// Build a [`Fixture`] from a Lua config snippet, panicking on parse errors.
///
/// Uses [`Config::parse_with_host`] so the returned fixture's scripting state carries the
/// Lua function callbacks registered by the snippet.
fn fixture_from_lua(snippet: &str) -> Fixture {
    let (config, host) = Config::parse_with_host(std::path::Path::new("test.lua"), snippet)
        .config
        .expect("lua snippet failed to parse");
    let mut fixture = Fixture::with_config(config);
    fixture.state.server.state.install_script_host(host);
    fixture
}

#[test]
fn bind_function_fires_when_key_pressed() {
    // `neori.action("create-virtual-output", { name = … })` is the observable side effect:
    // the virtual output appears only if the Lua function actually ran AND its queued
    // action was drained and dispatched by the compositor.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            neori.action("create-virtual-output", { name = "from-lua-bind" })
        end)
        "#,
    );

    // Build the Bind the Lua config produced, then hand it to handle_bind() just as the
    // input pipeline would after a key match.
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .expect("function bind missing")
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "from-lua-bind"),
        "neori.action inside a Lua bind should enqueue + dispatch"
    );
}

#[test]
fn release_bind_function_fires_on_release_only() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", {
            release = function()
                neori.action("create-virtual-output", { name = "from-lua-release" })
            end,
        })
        "#,
    );
    let created = |f: &mut Fixture| {
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "from-lua-release")
    };

    let bind = f.state.server.state.neori.config.borrow().binds.0[0].clone();

    // The press does nothing for a release-only bind.
    f.state
        .server
        .state
        .handle_bind(bind.clone(), true, InputTime::now());
    f.state.server.dispatch();
    assert!(!created(&mut f));

    // The release runs the function.
    f.state
        .server
        .state
        .handle_bind(bind, false, InputTime::now());
    f.state.server.dispatch();
    assert!(created(&mut f), "the release function should have run");
}

#[test]
fn bind_function_error_does_not_crash() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function() error("intentional") end)
        "#,
    );

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    // Must not panic; the error is caught and warn!()'d.
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();
}

/// A Lua log that the test can read back.
#[derive(Clone, Default)]
struct TestLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for TestLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TestLog {
    fn install(&self, f: &mut Fixture) {
        let sink: neori_config::PrintSink =
            std::sync::Arc::new(std::sync::Mutex::new(Box::new(self.clone())));
        let state = &mut f.state.server.state;
        state.scripting.set_print_log(std::sync::Arc::clone(&sink));
        state.print_log_sink = Some(sink);
    }

    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

#[test]
fn lua_errors_go_to_the_lua_log_with_their_own_notification() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            print("before")
            error("intentional")
        end)
        "#,
    );
    let log = TestLog::default();
    log.install(&mut f);

    press_first_function_bind(&mut f);

    // The error comes after what the callback printed, with its traceback.
    let text = log.text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with("] before"), "{text}");
    assert!(
        lines[1].contains("] error: Lua bind callback error: runtime error: ")
            && lines[1].contains("intentional"),
        "{text}"
    );
    assert!(text.contains("stack traceback"), "{text}");
    // Not the "failed to parse the config file" message: the config is fine.
    assert!(f.neori().config_error_notification.shows_lua_error());
}

#[test]
fn neori_action_on_event_takes_effect() {
    let mut f = fixture_from_lua(
        r#"
        neori.on("startup", function()
            neori.action("create-virtual-output", { name = "from-lua-startup" })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_state().fire_startup();
    assert!(has_output(&mut f, "from-lua-startup"));
}

#[test]
fn string_action_binds_still_work() {
    // Regression: the declarative-action path (string + table) must keep working alongside
    // Lua function binds.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", "create-virtual-output")
        "#,
    );

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config.binds.0[0].clone()
    };
    assert!(!matches!(
        bind.press_action(),
        Some(Action::ScriptFunction { .. })
    ));
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    // The no-arg "create-virtual-output" uses the HEADLESS-N counter.
    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name().starts_with("HEADLESS-")),
        "static Action::CreateVirtualOutput should still dispatch"
    );
}

#[test]
fn opening_a_window_fires_window_opened_event() {
    // window-opened handler creates "canary" virtual output as an observable side-effect.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-opened", function(w)
            seen_app_id = w:app_id() or "-"
            neori.action("create-virtual-output", { name = "canary" })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    window
        .xdg_toplevel
        .set_app_id("neori-test-opens".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "canary"),
        "window-opened should have fired and dispatched neori.action"
    );

    let app = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_app_id")
        .unwrap();
    let neori_config::mlua::Value::String(s) = app else {
        panic!("seen_app_id is not a string");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "neori-test-opens");
}

#[test]
fn neori_windows_returns_mapped_windows_from_bind_callback() {
    // Bind callback uses neori.windows() to iterate the live list.
    let mut f = fixture_from_lua(
        r#"
        count = 0
        neori.bind("Mod+Alt+T", function()
            for _, w in ipairs(neori.windows()) do
                count = count + 1
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    // Open two windows.
    for app in ["a", "b"] {
        let id = f.add_client();
        let window = f.client(id).create_window();
        window.xdg_toplevel.set_app_id(app.to_string());
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);
        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(200, 200);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
    }
    f.state.server.dispatch();

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let count = f.state.server.state.scripting.read_global("count").unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(2)),
        "expected count = 2, got {count:?}"
    );
}

#[test]
fn closing_a_window_fires_window_closed_event() {
    // Observable side-effects of the window-closed handler firing:
    //   1. A canary virtual output gets created via neori.action.
    //   2. Lua globals record the closed window's id and app_id.
    //
    // app_id is preserved across unmap by the `last_known_app_id` mirror on `Mapped`;
    // without that mirror it would read nil because the XDG role is cleared by the
    // client's unmap commit before the handler fires.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-closed", function(w)
            closed_id = w:id()
            closed_app_id = w:app_id() or "-"
            neori.action("create-virtual-output", { name = "epitaph" })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    // Open a window.
    let id = f.add_client();
    let window = f.client(id).create_window();
    window
        .xdg_toplevel
        .set_app_id("neori-test-closes".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let opened_id = f.neori().layout.focus().map(|m| m.id().get()).unwrap();

    // Unmap it (attach_null + commit — mirrors tests/floating.rs::unmap_from_floating).
    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    // Handler side-effect: canary output exists.
    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "epitaph"),
        "window-closed should have fired and dispatched neori.action"
    );

    // Handler saw the right window id.
    let seen_id = f
        .state
        .server
        .state
        .scripting
        .read_global("closed_id")
        .unwrap();
    assert!(
        matches!(seen_id, neori_config::mlua::Value::Integer(n) if n as u64 == opened_id),
        "window-closed handler got id {seen_id:?}, expected {opened_id}"
    );

    // Handler saw the app_id (via the `last_known_app_id` mirror on Mapped).
    let app = f
        .state
        .server
        .state
        .scripting
        .read_global("closed_app_id")
        .unwrap();
    let neori_config::mlua::Value::String(s) = app else {
        panic!("closed_app_id is not a string, got {app:?}");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "neori-test-closes");
}

#[test]
fn focusing_a_window_fires_window_focused_event() {
    // Open two windows back-to-back and count how many window-focused events fire.
    // Both opens focus their new window, so we expect two focus events.
    let mut f = fixture_from_lua(
        r#"
        focus_count = 0
        last_focused_app = nil
        neori.on("window-focused", function(w)
            focus_count = focus_count + 1
            last_focused_app = w:app_id() or "-"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    for app in ["first", "second"] {
        let id = f.add_client();
        let window = f.client(id).create_window();
        window.xdg_toplevel.set_app_id(app.to_string());
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);
        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(200, 200);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
    }
    f.state.server.dispatch();

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_count")
        .unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(n) if n >= 2),
        "expected at least 2 focus events, got {count:?}"
    );

    let app = f
        .state
        .server
        .state
        .scripting
        .read_global("last_focused_app")
        .unwrap();
    let neori_config::mlua::Value::String(s) = app else {
        panic!("last_focused_app is not a string");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "second");
}

#[test]
fn window_focused_debounces_on_identical_focus() {
    // Focus a window, then re-run the focus-update pass without changing focus. The
    // window-focused handler must NOT fire a second time because last_focused_window_id
    // is unchanged.
    let mut f = fixture_from_lua(
        r#"
        focus_count = 0
        neori.on("window-focused", function(w) focus_count = focus_count + 1 end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("solo".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let count_before = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_count")
        .unwrap();
    assert!(
        matches!(count_before, neori_config::mlua::Value::Integer(1)),
        "expected one initial focus event, got {count_before:?}"
    );

    // Trigger the focus pass again. Focus hasn't changed, so the event must not re-fire.
    for _ in 0..3 {
        f.state.server.state.update_keyboard_focus();
    }
    f.state.server.dispatch();

    let count_after = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_count")
        .unwrap();
    assert!(
        matches!(count_after, neori_config::mlua::Value::Integer(1)),
        "window-focused should not re-fire when focus is unchanged, got {count_after:?}"
    );
}

#[test]
fn window_close_method_closes_the_window() {
    // A Lua function bind that calls w:close() on the focused window should queue a
    // CloseWindow action with the window's id, which the compositor then dispatches —
    // closing the actual window.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            if w then w:close() end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("to-close".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    // The window is open.
    assert_eq!(f.neori().layout.windows().count(), 1);

    // Fire the bind. It enqueues Action::CloseWindow via neori.action-style path.
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    // The compositor sent a close request to the client. The client receives it,
    // acknowledges, and unmaps — roundtrip to drive that handshake.
    f.double_roundtrip(id);
    let window = f.client(id).window(&surface);
    assert!(
        window.close_requested,
        "Action::CloseWindow should have reached the client"
    );
}

#[test]
fn window_send_to_workspace_moves_the_window() {
    // Calling w:send_to_workspace(2) from a Lua bind should fire
    // Action::MoveWindowToWorkspace which puts the window on workspace 2.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            if w then w:send_to_workspace(2) end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    // Window starts on workspace 1.
    let initial_ws = f
        .neori()
        .layout
        .workspaces()
        .enumerate()
        .find_map(|(idx, (_, _, ws))| {
            if ws.windows().next().is_some() {
                Some(idx + 1)
            } else {
                None
            }
        })
        .expect("window should be on some workspace");
    assert_eq!(initial_ws, 1, "window should start on workspace 1");

    // Fire the bind. Action::MoveWindowToWorkspace runs.
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    // The window should now be on a workspace with idx >= 2.
    let after_ws = f
        .neori()
        .layout
        .workspaces()
        .enumerate()
        .find_map(|(idx, (_, _, ws))| {
            if ws.windows().next().is_some() {
                Some(idx + 1)
            } else {
                None
            }
        })
        .expect("window should still be on some workspace");
    assert!(
        after_ws >= 2,
        "window should have moved to workspace >= 2, still on {after_ws}"
    );
}

#[test]
fn window_focus_method_refocuses_a_window() {
    // Open two windows; the second steals focus. Then a Lua bind captures the first
    // Window from neori.windows() and calls :focus() — the compositor should move
    // focus back to the first window.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            for _, w in ipairs(neori.windows()) do
                if w:app_id() == "first" then w:focus() end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let mut ids_by_app = std::collections::HashMap::new();
    for app in ["first", "second"] {
        let id = f.add_client();
        let window = f.client(id).create_window();
        window.xdg_toplevel.set_app_id(app.to_string());
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);
        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(200, 200);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
        f.state.server.dispatch();
        ids_by_app.insert(app, f.neori().layout.focus().unwrap().id().get());
    }
    let first_id = ids_by_app["first"];
    let second_id = ids_by_app["second"];
    assert_ne!(first_id, second_id);
    assert_eq!(
        f.neori().layout.focus().unwrap().id().get(),
        second_id,
        "second window should have stolen focus on open",
    );

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    assert_eq!(
        f.neori().layout.focus().unwrap().id().get(),
        first_id,
        "w:focus() should have moved focus back to the first window"
    );
}

#[test]
fn window_fullscreen_method_sets_fullscreen_state() {
    // w:fullscreen(true) should drive the window into fullscreen; w:fullscreen(false)
    // should take it out. We observe this via Mapped::is_fullscreen() on the
    // compositor side, which is what the user sees through w:is_fullscreen() too.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+F", function()
            local w = neori.focused_window()
            if w then w:fullscreen(true) end
        end)
        neori.bind("Mod+Alt+G", function()
            local w = neori.focused_window()
            if w then w:fullscreen(false) end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    assert!(
        !f.neori().layout.focus().unwrap().is_fullscreen(),
        "window should start non-fullscreen"
    );

    // Fire w:fullscreen(true).
    let binds: Vec<_> = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .filter(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .cloned()
            .collect()
    };
    let (set_bind, unset_bind) = (binds[0].clone(), binds[1].clone());

    // Drive an ack+commit cycle: the compositor's FullscreenWindow dispatch sends a
    // configure, the client acks and commits with a new buffer, then the compositor's
    // post-commit path flips sizing_mode to fullscreen.
    let cycle = |f: &mut Fixture, id: ClientId, surface: &ClientWlSurface, bind: Bind| {
        f.state
            .server
            .state
            .handle_bind(bind, true, InputTime::now());
        f.state.server.dispatch();
        f.double_roundtrip(id);
        let window = f.client(id).window(surface);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
        f.state.server.dispatch();
    };
    cycle(&mut f, id, &surface, set_bind);
    assert!(
        f.neori().layout.focus().unwrap().is_fullscreen(),
        "w:fullscreen(true) should have set fullscreen"
    );
    cycle(&mut f, id, &surface, unset_bind);
    assert!(
        !f.neori().layout.focus().unwrap().is_fullscreen(),
        "w:fullscreen(false) should have cleared fullscreen"
    );
}

#[test]
fn window_float_method_moves_between_tiling_and_floating() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+F", function()
            local w = neori.focused_window()
            if w then w:float(true) end
        end)
        neori.bind("Mod+Alt+G", function()
            local w = neori.focused_window()
            if w then w:float(false) end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    assert!(
        !f.neori().layout.focus().unwrap().is_floating(),
        "window should start tiled"
    );

    let binds: Vec<_> = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .filter(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .cloned()
            .collect()
    };
    let (float_bind, tile_bind) = (binds[0].clone(), binds[1].clone());

    f.state
        .server
        .state
        .handle_bind(float_bind, true, InputTime::now());
    f.state.server.dispatch();
    f.double_roundtrip(id);
    assert!(
        f.neori().layout.focus().unwrap().is_floating(),
        "w:float(true) should have moved window to floating"
    );

    f.state
        .server
        .state
        .handle_bind(tile_bind, true, InputTime::now());
    f.state.server.dispatch();
    f.double_roundtrip(id);
    assert!(
        !f.neori().layout.focus().unwrap().is_floating(),
        "w:float(false) should have moved window back to tiling"
    );
}

#[test]
fn window_send_to_output_moves_the_window_between_outputs() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            if w then w:send_to_output("headless-2") end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));

    // Open a window; by default it lands on the focused output (headless-1).
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let window_id = f.neori().layout.focus().unwrap().id().get();
    let output_name_for_window = |f: &mut Fixture, win_id: u64| -> Option<String> {
        for (monitor, _, ws) in f.neori().layout.workspaces() {
            for w in ws.windows() {
                if w.id().get() == win_id {
                    return monitor.map(|m| m.output().name());
                }
            }
        }
        None
    };
    assert_eq!(
        output_name_for_window(&mut f, window_id).as_deref(),
        Some("headless-1"),
        "window should start on headless-1"
    );

    // Fire w:send_to_output("headless-2").
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();
    f.double_roundtrip(id);

    assert_eq!(
        output_name_for_window(&mut f, window_id).as_deref(),
        Some("headless-2"),
        "w:send_to_output should have moved the window to headless-2"
    );
}

#[test]
fn neori_outputs_returns_cached_outputs_from_bind_callback() {
    // Bind callback iterates neori.outputs() and writes a count + the focused output's
    // name to Lua globals. With two headless outputs the iteration should see two.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            out_count = #neori.outputs()
            focused_name = neori.focused_output() and neori.focused_output():name() or nil
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("out_count")
        .unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(2)),
        "expected 2 outputs, got {count:?}"
    );

    let focused = f
        .state
        .server
        .state
        .scripting
        .read_global("focused_name")
        .unwrap();
    let neori_config::mlua::Value::String(s) = focused else {
        panic!("focused_name is not a string");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "headless-1");
}

#[test]
fn output_set_scale_takes_effect_on_compositor_output() {
    // Lua bind calls out:set_scale(2.0) on the focused output. After dispatch the
    // compositor's Smithay Output should report scale = 2.0.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local out = neori.focused_output()
            if out then out:set_scale(2.0) end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    let before = f
        .neori()
        .global_space
        .outputs()
        .find(|o| o.name() == "headless-1")
        .unwrap()
        .current_scale()
        .fractional_scale();
    assert!(
        (before - 1.0).abs() < f64::EPSILON,
        "initial scale should be 1.0, got {before}"
    );

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let after = f
        .neori()
        .global_space
        .outputs()
        .find(|o| o.name() == "headless-1")
        .unwrap()
        .current_scale()
        .fractional_scale();
    assert!(
        (after - 2.0).abs() < f64::EPSILON,
        "scale should have become 2.0, got {after}"
    );
}

#[test]
fn workspace_set_name_and_unset_name_affect_layout() {
    // ws:set_name("code") should set the active workspace's name; ws:unset_name() clears it.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local ws = neori.focused_workspace()
            if ws then ws:set_name("code") end
        end)
        neori.bind("Mod+Alt+U", function()
            local ws = neori.focused_workspace()
            if ws then ws:unset_name() end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);
    f.state.server.dispatch();

    // Initially the active workspace is unnamed.
    let initial_name = f
        .neori()
        .layout
        .active_workspace()
        .and_then(|ws| ws.name().cloned());
    assert_eq!(initial_name, None);

    // Fire the first script bind (set_name).
    let (bind_set, bind_unset) = {
        let config = f.state.server.state.neori.config.borrow();
        let mut it = config
            .binds
            .0
            .iter()
            .filter(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })));
        let set = it.next().unwrap().clone();
        let unset = it.next().unwrap().clone();
        (set, unset)
    };
    f.state
        .server
        .state
        .handle_bind(bind_set, true, InputTime::now());
    f.state.server.dispatch();

    assert_eq!(
        f.neori()
            .layout
            .active_workspace()
            .and_then(|ws| ws.name().cloned()),
        Some("code".to_string()),
        "ws:set_name should have named the workspace"
    );

    // Fire the second script bind (unset_name).
    f.state
        .server
        .state
        .handle_bind(bind_unset, true, InputTime::now());
    f.state.server.dispatch();

    assert_eq!(
        f.neori()
            .layout
            .active_workspace()
            .and_then(|ws| ws.name().cloned()),
        None,
        "ws:unset_name should have cleared the name"
    );
}

#[test]
fn workspace_focus_method_activates_target_workspace() {
    // Move window to workspace 2 (which also activates it), then call ws:focus() on
    // workspace 1 from a Lua bind — the compositor should move the active workspace
    // back to 1.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            for _, ws in ipairs(neori.workspaces()) do
                if ws:idx() == 1 then ws:focus() end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    // Open one window so workspace 1 has content, then move it to workspace 2 to
    // create and activate workspace 2.
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let window_id = f.neori().layout.focus().unwrap().id();
    f.state.server.state.do_action(
        neori_config::Action::MoveWindowToWorkspace {
            window_id: Some(window_id.get()),
            reference: neori_ipc::WorkspaceReferenceArg::Index(2),
            focus: true,
        },
        false,
        InputTime::now(),
    );
    f.state.server.dispatch();
    f.double_roundtrip(id);

    // Active workspace should now be the second one.
    let active_before = {
        let active_id = f.neori().layout.active_workspace_id();
        f.neori()
            .layout
            .workspaces()
            .enumerate()
            .find_map(|(gi, (_, _, ws))| (active_id == Some(ws.id())).then_some(gi + 1))
    };
    assert_eq!(
        active_before,
        Some(2),
        "precondition: window move should have activated workspace 2"
    );

    // Fire the bind; ws:focus() on global idx 1.
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    // Active workspace should now be back to 1.
    let active_after = {
        let active_id = f.neori().layout.active_workspace_id();
        f.neori()
            .layout
            .workspaces()
            .enumerate()
            .find_map(|(gi, (_, _, ws))| (active_id == Some(ws.id())).then_some(gi + 1))
    };
    assert_eq!(
        active_after,
        Some(1),
        "ws:focus should have re-activated workspace 1"
    );
}

#[test]
fn workspace_send_to_output_moves_workspace_between_outputs() {
    // With two outputs, call ws:send_to_output on the active workspace of headless-1
    // to move it onto headless-2.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local ws = neori.focused_workspace()
            if ws then ws:send_to_output("headless-2") end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);
    f.state.server.dispatch();

    // Open a window so the active workspace has content and isn't scrubbed when
    // the move happens.
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let active_ws_id = f
        .neori()
        .layout
        .active_workspace_id()
        .expect("active workspace should exist");

    let find_output = |f: &mut Fixture, want_id| -> Option<String> {
        for (monitor, _, ws) in f.neori().layout.workspaces() {
            if ws.id() == want_id {
                return monitor.map(|m| m.output().name());
            }
        }
        None
    };
    assert_eq!(
        find_output(&mut f, active_ws_id).as_deref(),
        Some("headless-1"),
        "precondition: active workspace lives on headless-1"
    );

    // Fire ws:send_to_output("headless-2").
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();
    f.double_roundtrip(id);

    assert_eq!(
        find_output(&mut f, active_ws_id).as_deref(),
        Some("headless-2"),
        "ws:send_to_output should have moved the workspace to headless-2"
    );
}

#[test]
fn neori_workspaces_returns_cached_snapshots_from_bind_callback() {
    // With two outputs, at least one workspace exists per output, so neori.workspaces()
    // should see >= 2 workspaces, and focused_workspace():output() should name the
    // currently focused output.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            ws_count = #neori.workspaces()
            focused_output = neori.focused_workspace() and neori.focused_workspace():output() or nil
            focused_idx = neori.focused_workspace() and neori.focused_workspace():idx() or nil
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("ws_count")
        .unwrap();
    let neori_config::mlua::Value::Integer(n) = count else {
        panic!("ws_count is not an integer, got {count:?}");
    };
    assert!(n >= 2, "expected >= 2 workspaces with two outputs, got {n}");

    let focused_output = f
        .state
        .server
        .state
        .scripting
        .read_global("focused_output")
        .unwrap();
    let neori_config::mlua::Value::String(s) = focused_output else {
        panic!("focused_output is not a string, got {focused_output:?}");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "headless-1");

    // focused_idx should be a valid u8 index (1-based).
    let idx = f
        .state
        .server
        .state
        .scripting
        .read_global("focused_idx")
        .unwrap();
    let neori_config::mlua::Value::Integer(idx) = idx else {
        panic!("focused_idx is not an integer, got {idx:?}");
    };
    assert!(idx >= 1, "focused_idx should be 1-based, got {idx}");
}

#[test]
fn output_focused_event_fires_on_focus_change() {
    // Two outputs; focus moves between them. output-focused should fire each
    // time the active output changes.
    let mut f = fixture_from_lua(
        r#"
        focus_events = 0
        last_focused = nil
        neori.on("output-focused", function(out)
            focus_events = focus_events + 1
            last_focused = out:name()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);
    f.state.server.dispatch();
    f.neori_focus_output(2);
    f.state.server.dispatch();

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_events")
        .unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(n) if n >= 2),
        "expected at least 2 output-focused events, got {count:?}"
    );
    let last = f
        .state
        .server
        .state
        .scripting
        .read_global("last_focused")
        .unwrap();
    let neori_config::mlua::Value::String(s) = last else {
        panic!("last_focused is not a string");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "headless-2");
}

#[test]
fn workspace_focused_event_fires_when_active_workspace_changes() {
    // Move window to workspace 2 — the active workspace changes, so
    // workspace-focused should fire with the new workspace snapshot.
    let mut f = fixture_from_lua(
        r#"
        focus_events = 0
        last_focused_idx = nil
        neori.on("workspace-focused", function(ws)
            focus_events = focus_events + 1
            last_focused_idx = ws:idx()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    // Open a window so there's something to move.
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    // Baseline count from startup focus (workspace 1 was focused when outputs came up).
    let baseline = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_events")
        .unwrap();
    let neori_config::mlua::Value::Integer(baseline) = baseline else {
        panic!("focus_events is not an integer, got {baseline:?}");
    };

    let window_id = f.neori().layout.focus().unwrap().id();
    f.state.server.state.do_action(
        neori_config::Action::MoveWindowToWorkspace {
            window_id: Some(window_id.get()),
            reference: neori_ipc::WorkspaceReferenceArg::Index(2),
            focus: true,
        },
        false,
        InputTime::now(),
    );
    f.state.server.dispatch();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let after = f
        .state
        .server
        .state
        .scripting
        .read_global("focus_events")
        .unwrap();
    let neori_config::mlua::Value::Integer(after) = after else {
        panic!("focus_events is not an integer, got {after:?}");
    };
    assert!(
        after > baseline,
        "workspace-focused should have fired on move; baseline={baseline} after={after}"
    );

    let last_idx = f
        .state
        .server
        .state
        .scripting
        .read_global("last_focused_idx")
        .unwrap();
    let neori_config::mlua::Value::Integer(idx) = last_idx else {
        panic!("last_focused_idx is not an integer, got {last_idx:?}");
    };
    assert!(
        idx >= 2,
        "latest focused workspace idx should be >= 2 after move, got {idx}"
    );
}

#[test]
fn workspace_added_event_fires_when_new_workspace_appears() {
    // Opening a window on a not-yet-materialized workspace creates that workspace,
    // which should fire workspace-added.
    let mut f = fixture_from_lua(
        r#"
        added = 0
        last_added_idx = nil
        neori.on("workspace-added", function(ws)
            added = added + 1
            last_added_idx = ws:idx()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let baseline = f.state.server.state.scripting.read_global("added").unwrap();
    let neori_config::mlua::Value::Integer(baseline) = baseline else {
        panic!("added is not an integer, got {baseline:?}");
    };

    // Move the window to workspace 2 — this creates workspace 2.
    let window_id = f.neori().layout.focus().unwrap().id();
    f.state.server.state.do_action(
        neori_config::Action::MoveWindowToWorkspace {
            window_id: Some(window_id.get()),
            reference: neori_ipc::WorkspaceReferenceArg::Index(2),
            focus: true,
        },
        false,
        InputTime::now(),
    );
    f.state.server.dispatch();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let after = f.state.server.state.scripting.read_global("added").unwrap();
    let neori_config::mlua::Value::Integer(after) = after else {
        panic!("added is not an integer, got {after:?}");
    };
    assert!(
        after > baseline,
        "workspace-added should have fired on new-workspace creation; baseline={baseline} after={after}"
    );
}

#[test]
fn window_opening_event_fires_with_app_id_and_title() {
    // Opening a window should fire `window-opening` once with the pre-placement
    // Window userdata carrying the client-set app_id and title.
    let mut f = fixture_from_lua(
        r#"
        openings = 0
        last_app = nil
        neori.on("window-opening", function(w)
            openings = openings + 1
            last_app = w:app_id()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("firefox".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("openings")
        .unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(n) if n == 1),
        "expected exactly 1 window-opening event, got {count:?}"
    );
    let app = f
        .state
        .server
        .state
        .scripting
        .read_global("last_app")
        .unwrap();
    let neori_config::mlua::Value::String(s) = app else {
        panic!("last_app is not a string, got {app:?}");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "firefox");
}

#[test]
fn window_opening_open_fullscreen_takes_effect_before_first_paint() {
    // A window-opening handler calling `w:open_fullscreen(true)` must land the
    // window in fullscreen by the time placement completes — no toggle cycle,
    // no visible flash into tiled first.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-opening", function(w)
            if w:app_id() == "needs-full" then w:open_fullscreen(true) end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("needs-full".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    assert!(
        f.neori().layout.focus().unwrap().is_fullscreen(),
        "w:open_fullscreen(true) at window-opening should land the window fullscreen"
    );
}

#[test]
fn window_opening_open_on_output_lands_on_target() {
    // Two outputs. A window-opening handler sends windows whose app_id is
    // "goes-right" to the second output regardless of which output is active.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-opening", function(w)
            if w:app_id() == "goes-right" then w:open_on_output("headless-2") end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("goes-right".to_string());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    // Search all workspaces for the (single) mapped window — focus may not
    // follow a window that opens on a non-active output.
    let output_name = {
        let mut hit: Option<String> = None;
        for (monitor, _, ws) in f.neori().layout.workspaces() {
            if ws.windows().next().is_some() {
                hit = monitor.map(|m| m.output().name());
                break;
            }
        }
        hit
    };
    assert_eq!(
        output_name.as_deref(),
        Some("headless-2"),
        "w:open_on_output(\"headless-2\") at window-opening should place the window on headless-2"
    );
}

#[test]
fn pointer_cache_is_refreshed_before_bind_callback() {
    // A bind callback that reads neori.pointer() should see a non-nil table
    // reflecting the current seat pointer position.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local p = neori.pointer()
            if p then
                seen_x = p.x
                seen_y = p.y
                seen_output = p.output
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);
    f.state
        .server
        .state
        .move_cursor(smithay::utils::Point::from((640.0, 360.0)));
    f.state.server.dispatch();

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let x = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_x")
        .unwrap();
    let y = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_y")
        .unwrap();
    assert!(
        matches!(x, neori_config::mlua::Value::Number(n) if (n - 640.0).abs() < 1e-6),
        "expected seen_x ≈ 640.0, got {x:?}"
    );
    assert!(
        matches!(y, neori_config::mlua::Value::Number(n) if (n - 360.0).abs() < 1e-6),
        "expected seen_y ≈ 360.0, got {y:?}"
    );
    let output = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_output")
        .unwrap();
    let neori_config::mlua::Value::String(s) = output else {
        panic!("seen_output is not a string, got {output:?}");
    };
    assert_eq!(s.to_str().unwrap().to_string(), "headless-1");
}

#[test]
fn warp_moves_the_cursor() {
    // neori.warp(x, y) from a bind callback should move the seat pointer there
    // by the time control returns to the test.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            neori.warp(123.0, 456.0)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_focus_output(1);
    f.state
        .server
        .state
        .move_cursor(smithay::utils::Point::from((0.0, 0.0)));
    f.state.server.dispatch();

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    let pointer = f
        .state
        .server
        .state
        .neori
        .seat
        .get_pointer()
        .unwrap()
        .current_location();
    assert!(
        (pointer.x - 123.0).abs() < 1e-6,
        "expected x ≈ 123.0, got {}",
        pointer.x
    );
    assert!(
        (pointer.y - 456.0).abs() < 1e-6,
        "expected y ≈ 456.0, got {}",
        pointer.y
    );
}

#[test]
fn keyboard_layout_is_readable_from_lua() {
    // neori.keyboard_layout() reads the active xkb layout name.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            seen_layout = neori.keyboard_layout()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let layout = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_layout")
        .unwrap();
    let neori_config::mlua::Value::String(s) = layout else {
        panic!("seen_layout is not a string, got {layout:?}");
    };
    // The default xkb layout is en_US / "English (US)" — be lenient about the exact wording.
    let name = s.to_str().unwrap().to_string();
    assert!(!name.is_empty(), "keyboard layout name should be non-empty");
}

#[test]
fn devices_returns_empty_array_in_test_fixture() {
    // The test fixture doesn't bring up real libinput, so neori.devices() should
    // return an empty array. This exercises the cache priming / drain plumbing
    // and documents that Device userdata is wired correctly end-to-end.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+D", function()
            seen_count = #neori.devices()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .unwrap()
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("seen_count")
        .unwrap();
    assert!(
        matches!(count, neori_config::mlua::Value::Integer(0)),
        "expected 0 devices in the test fixture, got {count:?}"
    );
}

#[test]
fn handle_bind_ignores_unknown_script_function() {
    // Edge case: a bind to a Lua function id that the script host doesn't know (e.g. one
    // constructed by mistake) should be harmless, not a panic.
    let mut f = Fixture::new();
    use neori_config::binds::{Bind, BoundAction, Key, Modifiers, Trigger};
    let bind = Bind {
        key: Key {
            trigger: Trigger::Keysym(Keysym::XF86_Reload),
            modifiers: Modifiers::empty(),
        },
        action: BoundAction::Press(Action::ScriptFunction { id: u64::MAX }),
        repeat: true,
        cooldown: None,
        allow_when_locked: false,
        allow_inhibiting: true,
        pass_through: None,
        hotkey_overlay_title: None,
    };
    // Must not panic.
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();

    // Suppress unused-import warnings in restricted feature builds.
    let _ = KeyState::Pressed;
    let _ = ModifiersState::default();
}

/// Runs the first Lua-function bind of the fixture's config, like a key press would.
fn press_first_function_bind(f: &mut Fixture) {
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config
            .binds
            .0
            .iter()
            .find(|b| matches!(b.press_action(), Some(Action::ScriptFunction { .. })))
            .expect("function bind missing")
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();
}

fn read_global_string(f: &Fixture, name: &str) -> String {
    match f.state.server.state.scripting.read_global(name).unwrap() {
        neori_config::mlua::Value::String(s) => s.to_str().unwrap().to_string(),
        other => panic!("{name} is {other:?}"),
    }
}

fn has_output(f: &mut Fixture, name: &str) -> bool {
    f.neori().global_space.outputs().any(|o| o.name() == name)
}

#[test]
fn stashed_window_sees_live_title() {
    // A Window kept in a Lua variable reads the window's current title, not the one it
    // had when the Window was first handed to Lua.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            stashed = stashed or neori.focused_window()
            seen_title = stashed and stashed:title() or "-"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.set_title("first");
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "seen_title"), "first");

    let window = f.client(id).window(&surface);
    window.set_title("second");
    window.commit();
    f.double_roundtrip(id);

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "seen_title"), "second");
}

#[test]
fn startup_handler_actions_are_dispatched() {
    // Regression: actions queued by a `startup` handler used to be dropped.
    let mut f = fixture_from_lua(
        r#"
        neori.on("startup", function()
            neori.action("create-virtual-output", { name = "from-startup" })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    f.neori_state().fire_startup();
    f.state.server.dispatch();
    assert!(has_output(&mut f, "from-startup"));
}

#[test]
fn monitor_added_handler_actions_are_dispatched() {
    // Regression: actions queued by a `monitor-added` handler used to be dropped.
    let mut f = fixture_from_lua(
        r#"
        neori.on("monitor-added", function(name)
            if name ~= "from-monitor" then
                neori.action("create-virtual-output", { name = "from-monitor" })
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    // monitor-added fires from an idle callback.
    f.state.server.dispatch();
    f.state.server.dispatch();
    assert!(has_output(&mut f, "from-monitor"));
}

#[test]
fn workspace_event_handler_actions_are_dispatched() {
    // Events that already dispatched their actions keep doing so through the shared path.
    let mut f = fixture_from_lua(
        r#"
        neori.on("workspace-focused", function(ws)
            if not created then
                created = true
                neori.action("create-virtual-output", { name = "from-workspace" })
            end
        end)
        "#,
    );
    // The first output coming up focuses its workspace.
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();
    f.state.server.dispatch();
    assert!(has_output(&mut f, "from-workspace"));
}

#[test]
fn stashed_workspace_sees_live_name() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            stashed = stashed or neori.focused_workspace()
            seen_name = stashed:name() or "-"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "seen_name"), "-");

    f.neori_state().do_action(
        Action::SetWorkspaceName {
            name: "renamed".into(),
            workspace: None,
        },
        false,
        InputTime::now(),
    );
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "seen_name"), "renamed");
}

#[test]
fn stashed_output_sees_its_new_scale() {
    // The first press changes the scale and reads it back right away; the second press reads
    // it again through the same Output value, kept in a variable.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            if not stashed then
                stashed = neori.focused_output()
                stashed:set_scale(2)
            end
            seen_scale = stashed:scale()
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();

    press_first_function_bind(&mut f);
    let read_scale = |f: &Fixture| match f.state.server.state.scripting.read_global("seen_scale") {
        Ok(neori_config::mlua::Value::Number(n)) => n,
        Ok(neori_config::mlua::Value::Integer(n)) => n as f64,
        other => panic!("seen_scale is {other:?}"),
    };
    assert_eq!(read_scale(&f), 2.0);

    press_first_function_bind(&mut f);
    assert_eq!(read_scale(&f), 2.0);
}

/// Maps a new window with `title` and returns its surface.
fn map_window(f: &mut Fixture, id: crate::tests::client::ClientId, title: &str) -> ClientWlSurface {
    let window = f.client(id).create_window();
    window.set_title(title);
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    surface
}

#[test]
fn window_signals_reach_only_their_window() {
    // w:connect("focused") and w:connect("closed") fire for that window only, through the
    // compositor's own window-focused / window-closed events.
    let mut f = fixture_from_lua(
        r#"
        focused, closed = "", ""
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            w:connect("focused", function(win) focused = focused .. win:title() .. ";" end)
            w:connect("closed", function(win) closed = closed .. win:title() .. ";" end)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let first = map_window(&mut f, id, "first");
    let second = map_window(&mut f, id, "second");
    f.state.server.dispatch();

    // Connect to "second", which is focused.
    press_first_function_bind(&mut f);

    // Focusing "first" doesn't fire; focusing "second" again does.
    f.neori_state()
        .do_action(Action::FocusColumnLeft {}, false, InputTime::now());
    f.state.server.dispatch();
    f.neori_state()
        .do_action(Action::FocusColumnRight {}, false, InputTime::now());
    f.state.server.dispatch();
    assert_eq!(read_global_string(&f, "focused"), "second;");

    // Closing "first" doesn't reach second's handlers.
    let window = f.client(id).window(&first);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();
    assert_eq!(read_global_string(&f, "closed"), "");

    // Closing "second" does, with the title it last had.
    let window = f.client(id).window(&second);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();
    assert_eq!(read_global_string(&f, "closed"), "second;");
}

fn read_global_int(f: &Fixture, name: &str) -> i64 {
    match f.state.server.state.scripting.read_global(name).unwrap() {
        neori_config::mlua::Value::Integer(n) => n,
        other => panic!("{name} is {other:?}"),
    }
}

/// Runs `action` and lets idle callbacks (like monitor-added) and the per-tick IPC refresh
/// (workspace and output events) run.
fn act(f: &mut Fixture, action: Action) {
    f.neori_state().do_action(action, false, InputTime::now());
    f.state.server.dispatch();
    f.state.server.dispatch();
}

fn unmap_window(f: &mut Fixture, id: crate::tests::client::ClientId, surface: &ClientWlSurface) {
    let window = f.client(id).window(surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
    f.state.server.dispatch();
}

#[test]
fn connected_builtin_signal_fires_until_disconnected() {
    // neori.connect to a built-in signal fires on the real event; a bind disconnects it.
    let mut f = fixture_from_lua(
        r#"
        opened = 0
        local on_open = neori.connect("window-opened", function(w) opened = opened + 1 end)
        neori.bind("Mod+Alt+T", function() neori.disconnect("window-opened", on_open) end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    map_window(&mut f, id, "one");
    f.state.server.dispatch();
    assert_eq!(read_global_int(&f, "opened"), 1);

    press_first_function_bind(&mut f);
    map_window(&mut f, id, "two");
    f.state.server.dispatch();
    assert_eq!(read_global_int(&f, "opened"), 1);
}

#[test]
fn emit_from_a_bind_runs_handlers_and_their_actions() {
    // A custom signal emitted from a bind runs its handler inside the bind's callback, so the
    // handler's neori.action calls are dispatched too.
    let mut f = fixture_from_lua(
        r#"
        neori.connect("make-output", function(name)
            neori.action("create-virtual-output", { name = name })
        end)
        neori.bind("Mod+Alt+T", function() neori.emit("make-output", "from-emit") end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();

    press_first_function_bind(&mut f);
    assert!(has_output(&mut f, "from-emit"));
}

#[test]
fn output_signals_fire_on_hotplug_and_survive_replug() {
    let mut f = fixture_from_lua(
        r#"
        added, removed = 0, 0
        neori.bind("Mod+Alt+T", function()
            for _, out in ipairs(neori.outputs()) do
                if out:name() == "dock" then
                    out:connect("added", function(name, o) added = added + 1 end)
                    out:connect("removed", function(name, o)
                        removed = removed + 1
                        removed_name = name
                    end)
                end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let create = || Action::CreateVirtualOutput {
        name: Some("dock".into()),
        width: None,
        height: None,
        refresh_rate: None,
    };
    let remove = || Action::RemoveVirtualOutput {
        name: "dock".into(),
    };
    act(&mut f, create());
    assert!(has_output(&mut f, "dock"));
    press_first_function_bind(&mut f);

    act(&mut f, remove());
    assert_eq!(read_global_int(&f, "removed"), 1);
    assert_eq!(read_global_string(&f, "removed_name"), "dock");

    // Plugged back in: the same handlers still fire.
    act(&mut f, create());
    assert_eq!(read_global_int(&f, "added"), 1);
    act(&mut f, remove());
    assert_eq!(read_global_int(&f, "removed"), 2);
}

#[test]
fn output_focused_signal_fires_for_that_output_only() {
    let mut f = fixture_from_lua(
        r#"
        focused = 0
        neori.bind("Mod+Alt+T", function()
            for _, out in ipairs(neori.outputs()) do
                if out:name() == "headless-2" then
                    out:connect("focused", function(o) focused = focused + 1 end)
                end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.neori_focus_output(1);
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    act(&mut f, Action::FocusMonitorNext {});
    assert_eq!(
        f.neori().layout.active_output().unwrap().name(),
        "headless-2"
    );
    assert_eq!(read_global_int(&f, "focused"), 1);

    // Focusing the other output doesn't fire it.
    act(&mut f, Action::FocusMonitorNext {});
    assert_eq!(read_global_int(&f, "focused"), 1);
}

#[test]
fn workspace_signals_fire_for_that_workspace_only() {
    let mut f = fixture_from_lua(
        r#"
        focused = 0
        neori.bind("Mod+Alt+T", function()
            neori.focused_workspace():connect("focused", function(ws) focused = focused + 1 end)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    // A window keeps workspace 1 around while we look at another one.
    let id = f.add_client();
    map_window(&mut f, id, "keeps-ws-1");
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    let focus = |idx| Action::FocusWorkspace {
        reference: neori_ipc::WorkspaceReferenceArg::Index(idx),
    };
    act(&mut f, focus(2));
    assert_eq!(read_global_int(&f, "focused"), 0);
    act(&mut f, focus(1));
    assert_eq!(read_global_int(&f, "focused"), 1);
}

#[test]
fn workspace_removed_signal_fires_when_it_goes_away() {
    let mut f = fixture_from_lua(
        r#"
        removed = 0
        neori.bind("Mod+Alt+T", function()
            neori.focused_workspace():connect("removed", function(ws) removed = removed + 1 end)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "keeps-ws-1");
    f.state.server.dispatch();

    let focus = |idx| Action::FocusWorkspace {
        reference: neori_ipc::WorkspaceReferenceArg::Index(idx),
    };
    // Connect on an empty workspace, then leave it. Empty workspaces are cleaned up once the
    // switch animation is done.
    act(&mut f, focus(2));
    press_first_function_bind(&mut f);
    act(&mut f, focus(1));
    f.neori_complete_animations();
    f.state.server.dispatch();
    f.state.server.dispatch();
    assert!(f.neori().layout.workspaces().count() == 1);
    assert_eq!(read_global_int(&f, "removed"), 1);
    // The removed workspace's handlers are gone.
    assert!(!f.state.server.state.scripting.has_object_signals());
}

#[test]
fn failing_object_handler_does_not_stop_the_others() {
    let mut f = fixture_from_lua(
        r#"
        closed = 0
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            w:connect("closed", function() error("intentional") end)
            w:connect("closed", function() closed = closed + 1 end)
        end)
        neori.on("window-closed", function() closed = closed + 10 end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "w");
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    unmap_window(&mut f, id, &surface);
    assert_eq!(read_global_int(&f, "closed"), 11);
    // The closed window's handlers are gone.
    assert!(!f.state.server.state.scripting.has_object_signals());
}

#[test]
fn signal_list_tracks_handlers_through_real_events() {
    // neori.signals() counts the handlers of every signal; a closed window's handlers drop out.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-closed", function() end)
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            if w then w:connect("closed", function() end) end
            for _, s in ipairs(neori.signals()) do
                if s.name == "window-closed" then
                    summary = s.handlers .. "+" .. s.object_handlers
                end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "w");
    f.state.server.dispatch();

    // One global handler, plus the one the bind just connected to the window.
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "summary"), "1+1");

    // Once the window is closed, its handler is gone.
    unmap_window(&mut f, id, &surface);
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "summary"), "1+0");
}

fn focused_snapshot(f: &mut Fixture) -> neori_config::WindowSnapshot {
    let neori = f.neori();
    let mapped = neori.layout.focus().unwrap();
    crate::script::snapshot_mapped(neori, mapped)
}

#[test]
fn events_fired_while_lua_runs_are_delivered_afterwards_in_order() {
    // Nothing fires Lua events from inside a callback yet, so simulate it: mark Lua as running,
    // fire two events, and check that they wait for the "callback" to finish.
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("window-focused", function(w) log = log .. "focused;" end)
        neori.on("layout-changed", function(name) log = log .. name .. ";" end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "w");
    f.state.server.dispatch();
    let snap = focused_snapshot(&mut f);
    let before = read_global_string(&f, "log");

    let state = f.neori_state();
    state.with_lua_running(running(0), |state| {
        state.fire_window_event("window-focused", snap);
        state.fire_layout_changed_event("second".into());
    });
    assert_eq!(
        read_global_string(&f, "log"),
        before,
        "must not run while Lua runs"
    );

    f.neori_state().deliver_queued_lua_events();
    assert_eq!(
        read_global_string(&f, "log"),
        format!("{before}focused;second;")
    );
}

#[test]
fn queued_events_dispatch_their_actions() {
    let mut f = fixture_from_lua(
        r#"
        neori.on("layout-changed", function(name)
            neori.action("create-virtual-output", { name = name })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_state().with_lua_running(running(0), |state| {
        state.fire_layout_changed_event("from-queue".into());
    });
    assert!(!has_output(&mut f, "from-queue"));
    f.neori_state().deliver_queued_lua_events();
    assert!(has_output(&mut f, "from-queue"));
}

#[test]
fn too_deeply_caused_events_are_dropped_with_an_error() {
    let mut f = fixture_from_lua(
        r#"
        ran = 0
        neori.on("layout-changed", function() ran = ran + 1 end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let notification_shown =
        |f: &mut Fixture| f.neori().config_error_notification.are_animations_ongoing();
    assert!(!notification_shown(&mut f));

    // One level below the limit: still delivered.
    let max = crate::script::MAX_LUA_EVENT_DEPTH;
    f.neori_state().with_lua_running(running(max - 1), |state| {
        state.fire_layout_changed_event("ok".into());
    });
    f.neori_state().deliver_queued_lua_events();
    assert_eq!(read_global_int(&f, "ran"), 1);
    assert!(!notification_shown(&mut f));

    // At the limit: dropped, and the user is told.
    f.neori_state().with_lua_running(running(max), |state| {
        state.fire_layout_changed_event("too-deep".into());
    });
    f.neori_state().deliver_queued_lua_events();
    assert_eq!(read_global_int(&f, "ran"), 1);
    assert!(notification_shown(&mut f));
}

#[test]
fn window_opening_still_runs_right_away_while_lua_runs() {
    // window-opening's result is needed immediately, so it isn't queued.
    let mut f = fixture_from_lua(
        r#"
        neori.on("window-opening", function(w) w:open_floating(true) end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let overrides = f.neori_state().with_lua_running(running(0), |state| {
        state.fire_window_opening_event(neori_config::WindowSnapshot::default())
    });
    assert_eq!(overrides.open_floating, Some(true));
}

/// Marks Lua as running `depth` events deep, as if a callback were running.
fn running(depth: u32) -> crate::script::Running {
    crate::script::Running {
        depth,
        allow_when_locked: false,
        time: InputTime::now(),
    }
}

#[test]
fn changes_are_visible_in_the_same_callback() {
    // Changes made from Lua happen right away, so the next line of the callback reads them.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            local other
            for _, x in ipairs(neori.windows()) do
                if x:id() ~= w:id() then other = x end
            end
            local seen = {}
            w:fullscreen(true)
            seen[#seen + 1] = "fullscreen=" .. tostring(w:is_fullscreen())
            other:float(true)
            seen[#seen + 1] = "floating=" .. tostring(other:is_floating())
            other:focus()
            seen[#seen + 1] = "focused=" .. tostring(other:is_focused())
                .. "," .. tostring(w:is_focused())
                .. "," .. neori.focused_window():title()
            neori.action("focus-workspace", 2)
            seen[#seen + 1] = "ws=" .. neori.focused_workspace():idx()
            local ws = neori.focused_workspace()
            ws:set_name("named")
            seen[#seen + 1] = "name=" .. ws:name()
            local out = neori.focused_output()
            out:set_scale(2)
            seen[#seen + 1] = "scale=" .. out:scale()
            log = table.concat(seen, " ")
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    f.state.server.dispatch();

    press_first_function_bind(&mut f);
    assert_eq!(
        read_global_string(&f, "log"),
        "fullscreen=true floating=true focused=true,false,a ws=2 name=named scale=2.0"
    );
}

#[test]
fn fullscreen_true_twice_stays_fullscreen() {
    // w:fullscreen(true) checks the state the compositor asked for, so calling it again before
    // the app has caught up doesn't toggle the window back.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            w:fullscreen(true)
            w:fullscreen(true)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "w");
    f.state.server.dispatch();

    let wants_fullscreen = |f: &mut Fixture| {
        use crate::layout::LayoutElement;
        f.neori()
            .layout
            .focus()
            .unwrap()
            .pending_sizing_mode()
            .is_fullscreen()
    };
    press_first_function_bind(&mut f);
    assert!(wants_fullscreen(&mut f));
    // A second press, still before the app has acknowledged anything.
    press_first_function_bind(&mut f);
    assert!(wants_fullscreen(&mut f));
}

/// Lets `ms` milliseconds pass, then runs the event loop so that due timers fire.
fn wait_ms(f: &mut Fixture, ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
    f.state.server.dispatch();
    f.state.server.dispatch();
}

#[test]
fn after_fires_once() {
    let mut f = fixture_from_lua(
        r#"
        fired = 0
        neori.after(200, function(t) fired = fired + 1; still_running = t:is_running() end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.state.server.dispatch();
    // Far less than 200 ms since the config loaded, even on a slow machine.
    assert_eq!(read_global_int(&f, "fired"), 0, "too early");

    wait_ms(&mut f, 250);
    assert_eq!(read_global_int(&f, "fired"), 1);
    assert!(matches!(
        f.state
            .server
            .state
            .scripting
            .read_global("still_running")
            .unwrap(),
        neori_config::mlua::Value::Boolean(false)
    ));
    wait_ms(&mut f, 30);
    assert_eq!(
        read_global_int(&f, "fired"),
        1,
        "a one-shot timer fires once"
    );
}

#[test]
fn every_repeats_until_stopped() {
    let mut f = fixture_from_lua(
        r#"
        ticks = 0
        ticker = neori.every(5, function() ticks = ticks + 1 end)
        neori.bind("Mod+Alt+T", function() ticker:stop() end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    for _ in 0..4 {
        wait_ms(&mut f, 10);
    }
    let ticks = read_global_int(&f, "ticks");
    assert!(ticks >= 2, "should have repeated, got {ticks}");

    press_first_function_bind(&mut f);
    let stopped_at = read_global_int(&f, "ticks");
    for _ in 0..3 {
        wait_ms(&mut f, 10);
    }
    assert_eq!(read_global_int(&f, "ticks"), stopped_at);
}

#[test]
fn timer_callback_can_stop_and_restart_its_timer() {
    let mut f = fixture_from_lua(
        r#"
        ticks, restarts = 0, 0
        neori.every(5, function(t)
            ticks = ticks + 1
            if ticks == 3 then t:stop() end
        end)
        neori.after(5, function(t)
            restarts = restarts + 1
            if restarts < 3 then t:again() end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    for _ in 0..10 {
        wait_ms(&mut f, 10);
    }
    assert_eq!(read_global_int(&f, "ticks"), 3);
    assert_eq!(read_global_int(&f, "restarts"), 3);
}

#[test]
fn timer_callbacks_run_actions_and_survive_errors() {
    let mut f = fixture_from_lua(
        r#"
        ticks = 0
        neori.every(5, function()
            ticks = ticks + 1
            if ticks == 1 then
                neori.action("create-virtual-output", { name = "from-timer" })
            end
            error("intentional")
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    for _ in 0..4 {
        wait_ms(&mut f, 10);
    }
    assert!(has_output(&mut f, "from-timer"));
    assert!(
        read_global_int(&f, "ticks") >= 2,
        "an error must not stop a repeating timer"
    );
    assert!(f.neori().config_error_notification.are_animations_ongoing());
}

#[test]
fn reload_drops_the_old_configs_timers() {
    let mut f = fixture_from_lua(
        r#"
        neori.after(20, function() neori.action("create-virtual-output", { name = "old-timer" }) end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let (_config, host) =
        Config::parse_with_host(std::path::Path::new("test.lua"), "-- no timers\n")
            .config
            .unwrap();
    f.state.server.state.install_script_host(host);
    wait_ms(&mut f, 40);
    assert!(!has_output(&mut f, "old-timer"));
}

/// Runs the event loop so that the per-refresh property diff runs.
fn refresh(f: &mut Fixture) {
    f.state.server.dispatch();
    f.state.server.dispatch();
}

#[test]
fn window_title_changed_fires_with_the_old_title() {
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("window-title-changed", function(w, old)
            log = log .. tostring(old) .. "->" .. w:title() .. ";"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "first");
    refresh(&mut f);
    assert_eq!(read_global_string(&f, "log"), "", "opening isn't a change");

    for title in ["second", "third"] {
        let window = f.client(id).window(&surface);
        window.set_title(title);
        window.commit();
        f.double_roundtrip(id);
        refresh(&mut f);
    }
    assert_eq!(
        read_global_string(&f, "log"),
        "first->second;second->third;"
    );
}

#[test]
fn window_property_signals_reach_their_window() {
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            w:connect("fullscreen-changed", function(win, was)
                log = log .. "fullscreen:" .. tostring(was) .. "->" .. tostring(win:is_fullscreen()) .. ";"
            end)
            w:connect("floating-changed", function(win, was)
                log = log .. "floating:" .. tostring(was) .. "->" .. tostring(win:is_floating()) .. ";"
            end)
            w:connect("urgent-changed", function(win, was)
                log = log .. "urgent:" .. tostring(was) .. "->" .. tostring(win:is_urgent()) .. ";"
            end)
            w:connect("workspace-changed", function(win, old)
                log = log .. "workspace:" .. old.idx .. "->" .. win:workspace().idx .. ";"
            end)
            w:connect("title-changed", function() log = log .. "title;" end)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let _other = map_window(&mut f, id, "other");
    let _watched = map_window(&mut f, id, "watched");
    refresh(&mut f);
    press_first_function_bind(&mut f);
    refresh(&mut f);
    let watched = f.neori().layout.focus().unwrap().id().get();

    act(&mut f, Action::FullscreenWindow { id: Some(watched) });
    refresh(&mut f);
    act(&mut f, Action::FullscreenWindow { id: Some(watched) });
    refresh(&mut f);
    act(&mut f, Action::MoveWindowToFloating { id: Some(watched) });
    refresh(&mut f);
    // A focused window can't become urgent: focus the other one first.
    let other = f
        .neori()
        .layout
        .windows()
        .map(|(_, m)| m.id().get())
        .find(|&id| id != watched)
        .unwrap();
    act(&mut f, Action::FocusWindow { id: other });
    refresh(&mut f);
    act(&mut f, Action::SetWindowUrgent { id: watched });
    refresh(&mut f);
    act(
        &mut f,
        Action::MoveWindowToWorkspace {
            window_id: Some(watched),
            reference: neori_ipc::WorkspaceReferenceArg::Index(2),
            focus: false,
        },
    );
    refresh(&mut f);

    assert_eq!(
        read_global_string(&f, "log"),
        "fullscreen:false->true;fullscreen:true->false;floating:false->true;\
         urgent:false->true;workspace:1->2;"
    );
}

#[test]
fn workspace_name_and_output_mode_changed() {
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("workspace-name-changed", function(ws, old)
            log = log .. "name:" .. tostring(old) .. "->" .. tostring(ws:name()) .. ";"
        end)
        neori.on("output-mode-changed", function(out, old)
            log = log .. "mode:" .. out:name() .. ":" .. tostring(old) .. "->" .. tostring(out:mode()) .. ";"
        end)
        neori.bind("Mod+Alt+T", function()
            for _, out in ipairs(neori.outputs()) do
                if out:name() == "dock" then out:set_mode("1280x720@60") end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    act(
        &mut f,
        Action::CreateVirtualOutput {
            name: Some("dock".into()),
            width: None,
            height: None,
            refresh_rate: None,
        },
    );
    refresh(&mut f);

    act(
        &mut f,
        Action::SetWorkspaceName {
            name: "web".into(),
            workspace: None,
        },
    );
    refresh(&mut f);
    act(&mut f, Action::UnsetWorkspaceName { reference: None });
    refresh(&mut f);
    press_first_function_bind(&mut f);
    refresh(&mut f);

    assert_eq!(
        read_global_string(&f, "log"),
        "name:nil->web;name:web->nil;mode:dock:1920x1080@60.000->1280x720@60.000;"
    );
}

#[test]
fn property_diff_keeps_no_state_without_handlers() {
    // Nothing listens: no state. Connecting records a baseline without firing.
    let mut f = fixture_from_lua(
        r#"
        fired = 0
        neori.bind("Mod+Alt+T", function()
            neori.connect("window-floating-changed", function() fired = fired + 1 end)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "w");
    refresh(&mut f);
    assert!(f.state.server.state.lua_properties.is_empty());

    press_first_function_bind(&mut f);
    refresh(&mut f);
    assert!(!f.state.server.state.lua_properties.is_empty());
    assert_eq!(read_global_int(&f, "fired"), 0);

    let window = f.neori().layout.focus().unwrap().id().get();
    act(&mut f, Action::MoveWindowToFloating { id: Some(window) });
    refresh(&mut f);
    assert_eq!(read_global_int(&f, "fired"), 1);
}

#[test]
fn is_startup_ends_after_the_startup_handlers() {
    let mut f = fixture_from_lua(
        r#"
        neori.on("startup", function() in_startup = tostring(neori.is_startup()) end)
        neori.bind("Mod+Alt+T", function() later = tostring(neori.is_startup()) end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_state().fire_startup();
    assert_eq!(read_global_string(&f, "in_startup"), "true");
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "later"), "false");
}

#[test]
fn config_reloaded_fires_in_the_new_config() {
    let mut f = fixture_from_lua(
        r#"
        neori.on("config-reloaded", function() old_config_ran = true end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_state().fire_startup();

    let reloaded = neori_config::while_reloading(|| {
        Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"
            loading_saw = tostring(neori.is_startup())
            neori.on("config-reloaded", function()
                reloaded_saw = tostring(neori.is_startup())
                neori.action("create-virtual-output", { name = "after-reload" })
            end)
            "#,
        )
        .config
    })
    .unwrap();
    f.neori_state().reload_config(Ok(reloaded));

    // The new config's handler ran, with its action, and the old config's didn't.
    assert_eq!(read_global_string(&f, "loading_saw"), "false");
    assert_eq!(read_global_string(&f, "reloaded_saw"), "false");
    assert!(has_output(&mut f, "after-reload"));
    assert!(matches!(
        f.state
            .server
            .state
            .scripting
            .read_global("old_config_ran")
            .unwrap(),
        neori_config::mlua::Value::Nil
    ));
}

#[test]
fn shutdown_runs_its_handlers() {
    let mut f = fixture_from_lua(
        r#"
        neori.on("shutdown", function()
            said_goodbye = "yes"
            neori.action("create-virtual-output", { name = "goodbye" })
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.neori_state().fire_lua_lifecycle_event("shutdown");
    assert_eq!(read_global_string(&f, "said_goodbye"), "yes");
    assert!(has_output(&mut f, "goodbye"));
}

/// Presses the configured bind whose key is `key`, like a key press would.
fn press_bind_for(f: &mut Fixture, key: &str) {
    let wanted: neori_config::Key = key.parse().unwrap();
    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        let active = f.state.server.state.neori.active_mode.clone();
        let binds = match &active {
            Some(name) => &config.modes.iter().find(|m| &m.name == name).unwrap().binds,
            None => &config.binds,
        };
        binds
            .0
            .iter()
            .find(|b| b.key == wanted || b.key == wanted.into_keycode().unwrap_or(wanted))
            .unwrap_or_else(|| panic!("no bind for {key} in mode {active:?}"))
            .clone()
    };
    f.state
        .server
        .state
        .handle_bind(bind, true, InputTime::now());
    f.state.server.dispatch();
}

#[test]
fn keybinding_mode_replaces_the_normal_binds() {
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.mode("resize", function()
            neori.bind("Mod+T", function() log = log .. "mode-T;" end)
            neori.bind("Escape", "exit-mode")
        end)
        neori.bind("Mod+T", function() log = log .. "normal-T;" end)
        neori.bind("Mod+R", "enter-mode resize")
        neori.on("mode-changed", function(mode, old)
            log = log .. "changed:" .. tostring(old) .. "->" .. tostring(mode) .. ";"
            current = tostring(neori.current_mode())
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    press_bind_for(&mut f, "Mod+T");
    press_bind_for(&mut f, "Mod+R");
    assert_eq!(
        f.state.server.state.neori.active_mode.as_deref(),
        Some("resize")
    );
    assert_eq!(read_global_string(&f, "current"), "resize");
    press_bind_for(&mut f, "Mod+T");
    press_bind_for(&mut f, "Escape");
    assert_eq!(f.state.server.state.neori.active_mode, None);
    press_bind_for(&mut f, "Mod+T");

    assert_eq!(
        read_global_string(&f, "log"),
        "normal-T;changed:nil->resize;mode-T;changed:resize->nil;normal-T;"
    );
}

#[test]
fn make_binds_iter_uses_the_active_modes_binds() {
    // The real lookup that key presses use, not just the test helper above.
    let mut f = fixture_from_lua(
        r#"
        neori.mode("resize", function() neori.bind("Mod+T", "exit-mode") end)
        neori.bind("Mod+T", "close-window")
        neori.bind("Mod+R", "enter-mode resize")
        "#,
    );
    f.add_output(1, (1920, 1080));
    let key = f.state.server.state.neori.config.borrow().binds.0[0].key;
    let trigger = |f: &mut Fixture| {
        f.state
            .server
            .state
            .binds_for_tests()
            .into_iter()
            .find(|b| b.key == key)
            .and_then(|b| b.press_action().cloned())
    };
    assert_eq!(trigger(&mut f), Some(Action::CloseWindow { id: None }));
    act(
        &mut f,
        Action::EnterMode {
            name: "resize".into(),
        },
    );
    assert_eq!(trigger(&mut f), Some(Action::ExitMode {}));
    act(&mut f, Action::ExitMode {});
    assert_eq!(trigger(&mut f), Some(Action::CloseWindow { id: None }));
}

#[test]
fn reload_leaves_a_mode_that_is_gone() {
    let mut f = fixture_from_lua(
        r#"
        neori.mode("resize", function() neori.bind("Escape", "exit-mode") end)
        neori.bind("Mod+R", "enter-mode resize")
        "#,
    );
    f.add_output(1, (1920, 1080));
    act(
        &mut f,
        Action::EnterMode {
            name: "resize".into(),
        },
    );
    assert_eq!(
        f.state.server.state.neori.active_mode.as_deref(),
        Some("resize")
    );

    let reloaded = neori_config::while_reloading(|| {
        Config::parse_with_host(
            std::path::Path::new("test.lua"),
            r#"neori.on("mode-changed", function(mode, old) left = tostring(old) end)"#,
        )
        .config
    })
    .unwrap();
    f.neori_state().reload_config(Ok(reloaded));
    assert_eq!(f.state.server.state.neori.active_mode, None);
    assert_eq!(read_global_string(&f, "left"), "resize");

    // Entering an unknown mode is ignored.
    act(
        &mut f,
        Action::EnterMode {
            name: "resize".into(),
        },
    );
    assert_eq!(f.state.server.state.neori.active_mode, None);
}

#[test]
fn window_pid_geometry_and_window_at() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            local g = w:geometry()
            info = string.format("pid=%s w=%d h=%d", tostring(w:pid() ~= nil), g.width, g.height)
            on_screen = g.x ~= nil and g.y ~= nil
            local under = neori.window_at(g.x + g.width / 2, g.y + g.height / 2)
            found = under ~= nil and under:id() == w:id()
            nothing = neori.window_at(-10000, -10000) == nil
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "w");
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    use crate::layout::LayoutElement;
    let size = f.neori().layout.focus().unwrap().size();
    assert_eq!(
        read_global_string(&f, "info"),
        format!("pid=true w={} h={}", size.w, size.h)
    );
    let read_bool = |f: &Fixture, name: &str| {
        matches!(
            f.state.server.state.scripting.read_global(name).unwrap(),
            neori_config::mlua::Value::Boolean(true)
        )
    };
    assert!(read_bool(&f, "on_screen"));
    assert!(read_bool(&f, "found"));
    assert!(read_bool(&f, "nothing"));
}

#[test]
fn window_geometry_off_screen_has_no_position() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            for _, w in ipairs(neori.windows()) do
                if w:title() == "away" then
                    local g = w:geometry()
                    away = tostring(g.x) .. "," .. tostring(g.width > 0)
                end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "away");
    f.state.server.dispatch();
    let window = f.neori().layout.focus().unwrap().id().get();
    act(
        &mut f,
        Action::MoveWindowToWorkspace {
            window_id: Some(window),
            reference: neori_ipc::WorkspaceReferenceArg::Index(2),
            focus: false,
        },
    );
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "away"), "nil,true");
}

#[test]
fn window_maximize_and_kill_guard() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            w:maximize(true)
            maximized = tostring(w:is_maximized())
            w:maximize(true)            -- already maximized: stays
            still = tostring(w:is_maximized())
            w:center()
            -- The test client runs in the test process, so this is neori's own pid.
            ok, err = pcall(function() w:kill() end)
            err = tostring(err)
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "w");
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    assert_eq!(read_global_string(&f, "maximized"), "true");
    assert_eq!(read_global_string(&f, "still"), "true");
    let err = read_global_string(&f, "err");
    assert!(err.contains("refusing to kill process"), "{err}");
}

#[test]
fn output_geometry_output_at_and_windows() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local lines = {}
            for _, out in ipairs(neori.outputs()) do
                local g = out:geometry()
                lines[#lines + 1] = string.format("%s %d,%d %dx%d windows=%d",
                    out:name(), g.x, g.y, g.width, g.height, #out:windows())
            end
            table.sort(lines)
            info = table.concat(lines, "; ")
            local right = neori.output_at(1930, 10)
            at_right = right and right:name() or "nil"
            at_nowhere = tostring(neori.output_at(-50, -50))
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    f.neori_focus_output(1);
    let id = f.add_client();
    map_window(&mut f, id, "w");
    f.state.server.dispatch();
    press_first_function_bind(&mut f);

    // Outputs are placed left to right.
    assert_eq!(
        read_global_string(&f, "info"),
        "headless-1 0,0 1920x1080 windows=1; headless-2 1920,0 1280x720 windows=0"
    );
    assert_eq!(read_global_string(&f, "at_right"), "headless-2");
    assert_eq!(read_global_string(&f, "at_nowhere"), "nil");
}

#[test]
fn output_workarea_leaves_out_panels() {
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

    use crate::tests::client::LayerConfigureProps;

    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local a = neori.focused_output():workarea()
            area = string.format("%d,%d %dx%d", a.x, a.y, a.width, a.height)
            physical = tostring(neori.focused_output():physical_size())
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "area"), "0,0 1920x1080");

    // A 50 px bar at the top.
    let id = f.add_client();
    let layer = f.client(id).create_layer(None, Layer::Top, "bar");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        exclusive_zone: Some(50),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(1920, 50);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "area"), "0,50 1920x1030");
    // Headless outputs have no physical size.
    assert_eq!(read_global_string(&f, "physical"), "nil");
}

#[test]
fn device_added_and_removed_signals() {
    // Real libinput devices can't be made in tests, so fire the events the way
    // process_libinput_event does.
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("device-added", function(d)
            log = log .. "added:" .. d:name() .. ";"
            neori.action("create-virtual-output", { name = "for-" .. d:sysname() })
        end)
        neori.on("device-removed", function(d) log = log .. "removed:" .. d:sysname() .. ";" end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let device = neori_config::DeviceSnapshot {
        name: "Test Mouse".into(),
        sysname: "event9".into(),
        is_pointer: true,
        ..Default::default()
    };
    f.neori_state()
        .fire_device_event("device-added", device.clone());
    f.neori_state().fire_device_event("device-removed", device);
    assert_eq!(
        read_global_string(&f, "log"),
        "added:Test Mouse;removed:event9;"
    );
    assert!(has_output(&mut f, "for-event9"));
}

#[test]
fn setenv_changes_the_spawn_environment_until_reload() {
    // CHILD_ENV is shared by every test: use a name no other test uses.
    let name = format!("NEORI_TEST_SETENV_{}", std::process::id());
    let mut f = fixture_from_lua(&format!(
        r#"
        neori.bind("Mod+Alt+T", function()
            neori.setenv("{name}", "one")
            after_set = neori.getenv("{name}")
            neori.unsetenv("{name}")
            after_unset = tostring(neori.getenv("{name}"))
            neori.setenv("{name}", "two")
        end)
        "#
    ));
    f.add_output(1, (1920, 1080));
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "after_set"), "one");
    assert_eq!(read_global_string(&f, "after_unset"), "nil");

    let child_value = |name: &str| {
        let env = crate::utils::spawning::CHILD_ENV.read().unwrap();
        env.0
            .iter()
            .find(|v| v.name == name)
            .and_then(|v| v.value.clone())
    };

    // A reload puts back the config's environment. (Other tests reload in parallel, which also
    // resets CHILD_ENV, so the checks stay inside one callback or after a reset.)
    let reloaded = neori_config::while_reloading(|| {
        Config::parse_with_host(std::path::Path::new("test.lua"), "-- nothing\n").config
    })
    .unwrap();
    f.neori_state().reload_config(Ok(reloaded));
    assert_eq!(child_value(&name), None);
}

#[test]
fn layer_surface_signals_and_object() {
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

    use crate::tests::client::LayerConfigureProps;

    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("layer-added", function(ls)
            -- Listing layers from a handler must not deadlock on the layer map.
            log = log .. "added:" .. ls:namespace() .. " count=" .. #neori.layers() .. ";"
            info = string.format("%s %s %s pid=%s", ls:layer(), tostring(ls:output()),
                ls:keyboard_interactivity(), tostring(ls:pid() ~= nil))
            ls:connect("removed", function(l) log = log .. "own-removed:" .. l:namespace() .. ";" end)
            ok, err = pcall(function() ls:kill() end)
            err = tostring(err)
        end)
        neori.on("layer-removed", function(ls) log = log .. "removed:" .. ls:namespace() .. ";" end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let layer = f.client(id).create_layer(None, Layer::Top, "bar");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(1920, 50);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_eq!(read_global_string(&f, "log"), "added:bar count=1;");
    assert_eq!(
        read_global_string(&f, "info"),
        "top headless-1 none pid=true"
    );
    // The test client runs in the test process: kill() refuses to kill neori itself.
    assert!(read_global_string(&f, "err").contains("refusing to kill process"));

    // Unmapping with a null buffer removes it; its own handler runs and is dropped.
    let layer = f.client(id).layer(&surface);
    layer.attach_null();
    layer.commit();
    f.double_roundtrip(id);
    assert_eq!(
        read_global_string(&f, "log"),
        "added:bar count=1;removed:bar;own-removed:bar;"
    );
    assert!(!f.state.server.state.scripting.has_object_signals());
}

#[test]
fn layer_surface_close() {
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;

    use crate::tests::client::LayerConfigureProps;

    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            for _, ls in ipairs(neori.layers()) do ls:close() end
            closed = "yes"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let layer = f.client(id).create_layer(None, Layer::Top, "launcher");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    press_first_function_bind(&mut f);
    f.double_roundtrip(id);
    assert_eq!(read_global_string(&f, "closed"), "yes");
    assert!(
        f.client(id).layer(&surface).close_requested,
        "the client should be asked to close"
    );
}

#[test]
fn layer_surface_removed_after_its_output() {
    use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;

    use crate::tests::client::LayerConfigureProps;

    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("layer-removed", function(ls)
            log = log .. "removed:" .. ls:namespace() .. " output=" .. tostring(ls:output()) .. ";"
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let id = f.add_client();
    f.double_roundtrip(id);
    let wl_output = f.client(id).output("headless-2");
    let layer = f
        .client(id)
        .create_layer(Some(&wl_output), Layer::Top, "bar");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
    assert_eq!(f.neori().mapped_layer_surfaces.len(), 1);

    // Removing the output asks the surface to close; the app then destroys it.
    let output = f.neori_output(2);
    f.neori().remove_output(&output);
    f.double_roundtrip(id);
    let layer = f.client(id).layer(&surface);
    assert!(layer.close_requested);
    layer.layer_surface.destroy();
    f.double_roundtrip(id);

    assert_eq!(read_global_string(&f, "log"), "removed:bar output=nil;");
    assert!(f.neori().mapped_layer_surfaces.is_empty());
}

#[test]
fn window_opacity_overrides_and_resets_to_the_window_rule() {
    let mut f = fixture_from_lua(
        r#"
        neori.window_rule { matches = { { title = "^dim$" } }, opacity = 0.8 }
        log = ""
        step = 0
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            step = step + 1
            if step == 1 then
                log = log .. string.format("%.2f;", w:opacity())
                w:set_opacity(0.5)
                log = log .. string.format("%.2f;", w:opacity())
                ok, err = pcall(function() w:set_opacity(2) end)
                err = tostring(err)
            else
                w:set_opacity(nil)
                log = log .. string.format("%.2f;", w:opacity())
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "dim");

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "log"), "0.80;0.50;");
    assert!(read_global_string(&f, "err").contains("between 0 and 1"));

    // The override survives a rule recompute: the new title no longer matches the rule.
    let window = f.client(id).window(&surface);
    window.set_title("bright");
    window.commit();
    f.double_roundtrip(id);
    use crate::layout::LayoutElement as _;
    let state = &mut f.state.server.state;
    let window = state.neori.layout.windows().next().unwrap().1;
    assert_eq!(window.rules().opacity, Some(0.5));

    // Resetting goes back to the rules, which no longer match.
    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "log"), "0.80;0.50;1.00;");
}

#[test]
fn window_parent_returns_the_parent_window() {
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", function()
            local w = neori.focused_window()
            local p = w:parent()
            result = w:title() .. " -> " .. (p and p:title() or "nil")
                .. "; " .. tostring(p and p:parent())
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let parent = map_window(&mut f, id, "main");
    let child = f.client(id).create_window();
    child.set_title("dialog");
    let child_surface = child.surface.clone();
    let parent_toplevel = f.client(id).window(&parent).xdg_toplevel.clone();
    let child = f.client(id).window(&child_surface);
    child.set_parent(Some(&parent_toplevel));
    child.commit();
    f.roundtrip(id);
    let child = f.client(id).window(&child_surface);
    child.attach_new_buffer();
    child.set_size(100, 100);
    child.ack_last_and_commit();
    f.double_roundtrip(id);

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "result"), "dialog -> main; nil");
}

#[test]
fn device_map_to_output_until_reset_or_reload() {
    // Real libinput devices can't be made in tests: map a synthetic one and check the stored
    // mapping, which the tablet and touch code reads by sysname.
    let mut f = fixture_from_lua(
        r#"
        log = ""
        neori.on("device-added", function(d)
            log = log .. tostring(d:mapped_output()) .. ";"
            d:map_to_output("headless-2")
            log = log .. tostring(d:mapped_output()) .. ";"
            saved = d
        end)
        neori.bind("Mod+Alt+T", function()
            saved:map_to_output(nil)
            log = log .. tostring(saved:mapped_output()) .. ";"
            saved:map_to_output("headless-1")
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let device = neori_config::DeviceSnapshot {
        name: "Test Tablet".into(),
        sysname: "event7".into(),
        ..Default::default()
    };
    f.neori_state().fire_device_event("device-added", device);
    assert_eq!(read_global_string(&f, "log"), "nil;headless-2;");
    assert_eq!(
        f.neori()
            .device_output_maps
            .get("event7")
            .map(String::as_str),
        Some("headless-2")
    );

    press_first_function_bind(&mut f);
    assert_eq!(read_global_string(&f, "log"), "nil;headless-2;nil;");
    assert_eq!(
        f.neori()
            .device_output_maps
            .get("event7")
            .map(String::as_str),
        Some("headless-1")
    );

    // The mapping belongs to the Lua state, which a reload replaces.
    let reloaded = neori_config::while_reloading(|| {
        Config::parse_with_host(std::path::Path::new("test.lua"), "-- nothing\n").config
    })
    .unwrap();
    f.neori_state().reload_config(Ok(reloaded));
    assert!(f.neori().device_output_maps.is_empty());
}

fn send_button(f: &mut Fixture, code: u32, pressed: bool) {
    use smithay::backend::input::{ButtonState, InputEvent};

    use crate::tests::test_input_backend::{TestInputBackend, TestPointerButtonEvent};

    let state = if pressed {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    };
    let event = TestPointerButtonEvent { code, state };
    f.neori_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerButton { event });
    f.state.server.dispatch();
}

fn send_gesture(
    f: &mut Fixture,
    make: impl FnOnce(
        crate::tests::test_input_backend::TestGestureEvent,
    ) -> smithay::backend::input::InputEvent<
        crate::tests::test_input_backend::TestInputBackend,
    >,
    event: crate::tests::test_input_backend::TestGestureEvent,
) {
    f.neori_state().process_input_event(make(event));
    f.state.server.dispatch();
}

#[test]
fn pointer_and_gesture_signals_fire_with_event_tables() {
    use smithay::backend::input::InputEvent;

    use crate::tests::test_input_backend::{
        TestGestureEvent, TestInputBackend, TestPointerAxisEvent,
    };

    let mut f = fixture_from_lua(
        r#"
        log = ""
        local function add(s) log = log .. s .. ";" end
        neori.on("pointer-button", function(e)
            add(string.format("button %d %s %s", e.button, e.name, tostring(e.pressed)))
        end)
        neori.on("pointer-axis", function(e)
            add(string.format("axis %s %g %g", e.source, e.dy_v120, e.dy))
        end)
        neori.on("gesture-swipe-begin", function(e) add("swipe-begin " .. e.fingers) end)
        neori.on("gesture-swipe-update", function(e) add(string.format("swipe %g %g", e.dx, e.dy)) end)
        neori.on("gesture-swipe-end", function(e) add("swipe-end " .. tostring(e.cancelled)) end)
        neori.on("gesture-pinch-begin", function(e) add("pinch-begin " .. e.fingers) end)
        neori.on("gesture-pinch-update", function(e)
            add(string.format("pinch %g %g", e.scale, e.rotation))
        end)
        neori.on("gesture-pinch-end", function(e) add("pinch-end " .. tostring(e.cancelled)) end)
        neori.on("gesture-hold-begin", function(e) add("hold-begin " .. e.fingers) end)
        neori.on("gesture-hold-end", function(e) add("hold-end " .. tostring(e.cancelled)) end)
        "#,
    );
    f.add_output(1, (1920, 1080));

    send_button(&mut f, 0x110, true);
    send_button(&mut f, 0x110, false);
    send_button(&mut f, 0x200, true);

    let event = TestPointerAxisEvent {
        horizontal_v120: 0.,
        vertical_v120: 120.,
    };
    f.neori_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerAxis { event });

    let g = TestGestureEvent::default;
    send_gesture(
        &mut f,
        |event| InputEvent::GestureSwipeBegin { event },
        TestGestureEvent { fingers: 5, ..g() },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GestureSwipeUpdate { event },
        TestGestureEvent {
            dx: 1.5,
            dy: -2.,
            ..g()
        },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GestureSwipeEnd { event },
        TestGestureEvent {
            cancelled: true,
            ..g()
        },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GesturePinchBegin { event },
        TestGestureEvent { fingers: 2, ..g() },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GesturePinchUpdate { event },
        TestGestureEvent {
            scale: 1.25,
            rotation: 10.,
            ..g()
        },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GesturePinchEnd { event },
        TestGestureEvent::default(),
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GestureHoldBegin { event },
        TestGestureEvent { fingers: 3, ..g() },
    );
    send_gesture(
        &mut f,
        |event| InputEvent::GestureHoldEnd { event },
        TestGestureEvent::default(),
    );

    // Button 0x200 has no name.
    let log = read_global_string(&f, "log");
    assert_eq!(
        log,
        "button 272 left true;button 272 left false;button 512 nil true;axis wheel 120 15;\
         swipe-begin 5;swipe 1.5 -2;swipe-end true;pinch-begin 2;pinch 1.25 10;\
         pinch-end false;hold-begin 3;hold-end false;"
    );
}

#[test]
fn begin_move_and_resize_need_a_held_button_and_end_on_release() {
    use crate::input::move_grab::MoveGrab;
    use crate::input::resize_grab::ResizeGrab;

    let mut f = fixture_from_lua(
        r#"
        presses = 0
        errors = ""
        neori.on("pointer-button", function(e)
            local w = neori.focused_window()
            if e.pressed then presses = presses + 1 end
            local ok, err = pcall(function()
                if presses == 1 then w:begin_move()
                elseif presses == 2 then w:begin_resize("bottom-right")
                else w:begin_resize("sideways") end
            end)
            if not ok then errors = errors .. tostring(err) .. ";" end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "win");

    let grab_is = |f: &mut Fixture, check: fn(&dyn std::any::Any) -> bool| {
        let pointer = f.neori().seat.get_pointer().unwrap();
        pointer
            .with_grab(|_, grab| check(grab.as_any()))
            .unwrap_or(false)
    };
    let is_move: fn(&dyn std::any::Any) -> bool = |g| g.downcast_ref::<MoveGrab>().is_some();
    let is_resize: fn(&dyn std::any::Any) -> bool = |g| g.downcast_ref::<ResizeGrab>().is_some();

    // Pressed: the move starts. Released: it ends, and begin_move() without a held button fails.
    send_button(&mut f, 0x110, true);
    assert!(grab_is(&mut f, is_move));
    send_button(&mut f, 0x110, false);
    assert!(!f.neori().seat.get_pointer().unwrap().is_grabbed());
    let errors = read_global_string(&f, "errors");
    assert!(errors.contains("needs a held mouse button"), "{errors}");

    send_button(&mut f, 0x110, true);
    assert!(grab_is(&mut f, is_resize));
    send_button(&mut f, 0x110, false);
    assert!(!f.neori().seat.get_pointer().unwrap().is_grabbed());

    // Bad edges: nothing starts; the press goes to the window as usual.
    send_button(&mut f, 0x110, true);
    assert!(!grab_is(&mut f, is_move) && !grab_is(&mut f, is_resize));
    let errors = read_global_string(&f, "errors");
    assert!(errors.contains("unknown edges \"sideways\""), "{errors}");
}

#[test]
fn begin_move_from_a_mouse_bind_ends_on_release() {
    // The bind suppresses the button; starting a move lets its release through to end it.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("MouseLeft", function() neori.focused_window():begin_move() end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "win");

    send_button(&mut f, 0x110, true);
    assert!(f.neori().seat.get_pointer().unwrap().is_grabbed());
    send_button(&mut f, 0x110, false);
    assert!(!f.neori().seat.get_pointer().unwrap().is_grabbed());
}

/// Presses and releases an unbound key, like the user typing.
fn type_key(f: &mut Fixture) {
    use smithay::backend::input::{InputEvent, Keycode};

    use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

    for state in [KeyState::Pressed, KeyState::Released] {
        f.neori_state()
            .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                event: TestKeyboardKeyEvent {
                    time: InputTime::from_micros(0),
                    // F12, which no bind uses.
                    code: Keycode::new(96),
                    state,
                    count: 1,
                },
            });
    }
}

#[test]
fn handlers_that_keep_undoing_each_other_across_refreshes_are_stopped() {
    // window-focused fires from the per-refresh focus check, after the handler that moved focus
    // returned, so the queue depth alone never saw this loop: focus bounced on every refresh.
    let mut f = fixture_from_lua(
        r#"
        runs = 0
        neori.on("window-focused", function(w)
            runs = runs + 1
            for _, other in ipairs(neori.windows()) do
                if other:id() ~= w:id() then
                    other:focus()
                    return
                end
            end
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    for _ in 0..100 {
        refresh(&mut f);
    }
    let runs = read_global_int(&f, "runs");
    assert!(
        runs <= i64::from(crate::script::MAX_LUA_EVENT_DEPTH) + 2,
        "the loop should stop at the depth limit, ran {runs} times"
    );

    // It stays stopped.
    for _ in 0..20 {
        refresh(&mut f);
    }
    assert_eq!(read_global_int(&f, "runs"), runs);
}

#[test]
fn a_handler_that_changes_something_on_every_focus_change_isnt_a_loop() {
    // The user moves focus on every refresh (holding a key), and the handler changes something
    // each time. Each focus change follows a key press, so it starts over at depth 0.
    let mut f = fixture_from_lua(
        r#"
        runs = 0
        neori.on("window-focused", function(w)
            runs = runs + 1
            neori.action("center-column")
        end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "a");
    map_window(&mut f, id, "b");
    refresh(&mut f);
    let ids: Vec<u64> = f
        .neori()
        .layout
        .windows()
        .map(|(_, w)| w.id().get())
        .collect();
    let before = read_global_int(&f, "runs");

    // One refresh per focus change, as when a held key repeats.
    for i in 0..40 {
        type_key(&mut f);
        f.neori_state().do_action(
            Action::FocusWindow {
                id: ids[i % ids.len()],
            },
            false,
            InputTime::now(),
        );
        f.state.server.dispatch();
    }
    assert_eq!(read_global_int(&f, "runs"), before + 40);
}

#[test]
fn neori_msg_lua_applies_config_changes_and_reports_bad_ones() {
    let mut f = fixture_from_lua(
        r#"
        neori.strict(true)
        layout = { gaps = 8 }
        neori.set("layout", layout)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let gaps = |f: &mut Fixture| f.neori().config.borrow().layout.gaps;
    assert_eq!(gaps(&mut f), 8.);

    // A write into a table that the config holds, which no declaration sees: `neori msg lua`
    // parses the config again after any code.
    f.neori_state().eval_lua("layout.gaps = 20").unwrap();
    assert_eq!(gaps(&mut f), 20.);

    // A change that doesn't parse is an error of the command, and is undone.
    let err = f
        .neori_state()
        .eval_lua(r#"neori.layout { gaps = "wide" }"#)
        .unwrap_err();
    assert!(err.contains("Lua config change undone: "), "{err}");
    assert_eq!(gaps(&mut f), 20.);
    assert_eq!(
        f.neori_state()
            .eval_lua("neori.layout.gaps")
            .unwrap()
            .values,
        ["20"]
    );
}
