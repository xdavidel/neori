//! Tests for the Lua scripting runtime API: function callbacks on binds, `neori.action`,
//! and error containment.

use neori_config::binds::Bind;
use neori_config::Config;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keysym, ModifiersState};
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;

use super::client::ClientId;
use super::*;

/// Build a [`Fixture`] from a Lua config snippet, panicking on parse errors.
///
/// Uses [`Config::parse_with_host`] so the returned fixture's scripting state carries the
/// Lua function callbacks registered by the snippet.
fn fixture_from_lua(snippet: &str) -> Fixture {
    let (config, host) =
        Config::parse_with_host(std::path::Path::new("test.lua"), snippet)
            .config
            .expect("lua snippet failed to parse");
    let mut fixture = Fixture::with_config(config);
    fixture.state.server.state.scripting = host;
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
            .find(|b| b.script_fn.is_some())
            .expect("function bind missing")
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    // Must not panic; the error is caught and warn!()'d.
    f.state.server.state.handle_bind(bind);
    f.state.server.dispatch();
}

#[test]
fn neori_action_on_event_fires_and_dispatches() {
    // Lua handler on a monitor event enqueues neori.action; after the event fires and the
    // pending queue is drained, the action should have taken effect.
    //
    // We drive this at a layer below the real Neori::add_output path so the test stays
    // deterministic: call call_event + drain explicitly.
    let f = fixture_from_lua(
        r#"
        neori.on("startup", function()
            neori.action("create-virtual-output", { name = "from-lua-startup" })
        end)
        "#,
    );

    let host = &f.state.server.state.scripting;
    let actions = host.call_event("startup").expect("startup event failed");
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0],
        neori_ipc::Action::CreateVirtualOutput { .. }
    ));
}

#[test]
fn string_action_binds_still_work() {
    // Regression: the existing declarative-action path (string + table) must keep working
    // after swapping Bind.action to Option<Action>.
    let mut f = fixture_from_lua(
        r#"
        neori.bind("Mod+Alt+T", "create-virtual-output")
        "#,
    );

    let bind = {
        let config = f.state.server.state.neori.config.borrow();
        config.binds.0[0].clone()
    };
    assert!(bind.script_fn.is_none());
    assert!(bind.action.is_some());
    f.state.server.state.handle_bind(bind);
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
    window.xdg_toplevel.set_app_id("neori-test-opens".to_string());
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);

    let count = f
        .state
        .server
        .state
        .scripting
        .read_global("count")
        .unwrap();
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            if ws
                .windows()
                .next()
                .is_some()
            {
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .filter(|b| b.script_fn.is_some())
            .cloned()
            .collect()
    };
    let (set_bind, unset_bind) = (binds[0].clone(), binds[1].clone());

    // Drive an ack+commit cycle: the compositor's FullscreenWindow dispatch sends a
    // configure, the client acks and commits with a new buffer, then the compositor's
    // post-commit path flips sizing_mode to fullscreen.
    let cycle = |f: &mut Fixture, id: ClientId, surface: &ClientWlSurface, bind: Bind| {
        f.state.server.state.handle_bind(bind);
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
            .filter(|b| b.script_fn.is_some())
            .cloned()
            .collect()
    };
    let (float_bind, tile_bind) = (binds[0].clone(), binds[1].clone());

    f.state.server.state.handle_bind(float_bind);
    f.state.server.dispatch();
    f.double_roundtrip(id);
    assert!(
        f.neori().layout.focus().unwrap().is_floating(),
        "w:float(true) should have moved window to floating"
    );

    f.state.server.state.handle_bind(tile_bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);

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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);

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
        let mut it = config.binds.0.iter().filter(|b| b.script_fn.is_some());
        let set = it.next().unwrap().clone();
        let unset = it.next().unwrap().clone();
        (set, unset)
    };
    f.state.server.state.handle_bind(bind_set);
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
    f.state.server.state.handle_bind(bind_unset);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);
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
            .find(|b| b.script_fn.is_some())
            .unwrap()
            .clone()
    };
    f.state.server.state.handle_bind(bind);

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

    let baseline = f
        .state
        .server
        .state
        .scripting
        .read_global("added")
        .unwrap();
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
    );
    f.state.server.dispatch();
    f.double_roundtrip(id);
    f.state.server.dispatch();

    let after = f
        .state
        .server
        .state
        .scripting
        .read_global("added")
        .unwrap();
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
fn handle_bind_ignores_bind_with_only_none_action() {
    // Edge case: a Bind { action: None, script_fn: None } should be harmless (no-op), not
    // a panic, if one ever gets constructed by mistake.
    let mut f = Fixture::new();
    use neori_config::binds::{Bind, Key, Modifiers, Trigger};
    let bind = Bind {
        key: Key {
            trigger: Trigger::Keysym(Keysym::XF86_Reload),
            modifiers: Modifiers::empty(),
        },
        action: None,
        script_fn: None,
        repeat: true,
        cooldown: None,
        allow_when_locked: false,
        allow_inhibiting: true,
        hotkey_overlay_title: None,
    };
    // Must not panic.
    f.state.server.state.handle_bind(bind);
    f.state.server.dispatch();

    // Suppress unused-import warnings in restricted feature builds.
    let _ = KeyState::Pressed;
    let _ = ModifiersState::default();
}
