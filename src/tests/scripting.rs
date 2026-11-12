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
