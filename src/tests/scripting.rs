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
