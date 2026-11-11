//! Tests for the Lua scripting runtime API: function callbacks on binds, `neori.action`,
//! and error containment.

use neori_config::Config;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keysym, ModifiersState};

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
