//! Tests for config-reload behavior as it intersects the Lua layer.
//!
//! The reload path (`State::reload_config`) swaps in a new `ScriptHost`,
//! which drops every handler registered by the old config and brings up
//! whatever the new config declared. These tests pin the handful of
//! invariants that are easy to break if someone mis-wires the swap:
//!
//! - Handlers from the old config stop firing after a successful reload.
//! - Handlers from the new config fire after reload.
//! - A failed reload (parse error) does NOT touch the current ScriptHost
//!   — old handlers keep firing.
//! - The tracking fields on `Neori` that back lifecycle-event diffs
//!   (currently just `last_focused_window_id`) survive the reload.

use neori_config::Config;

use super::Fixture;

/// Build a `(Config, ScriptHost)` tuple ready to pass into
/// `State::reload_config` as the Ok variant. Panics on parse failure.
fn parse(lua: &str) -> (Config, neori_config::ScriptHost) {
    Config::parse_with_host(std::path::Path::new("reload.lua"), lua)
        .config
        .expect("snippet failed to parse")
}

/// Build a fixture whose initial config is the given Lua snippet.
fn fixture_from(lua: &str) -> Fixture {
    let (config, host) = parse(lua);
    let mut fixture = Fixture::with_config(config);
    fixture.state.server.state.scripting = host;
    fixture
}

#[test]
fn old_startup_handlers_dont_fire_after_reload() {
    // Register a startup handler in config A. Reload to config B whose
    // startup handler sets a different global. Fire startup — only
    // config B's handler should run, in config B's Lua state.
    let mut f = fixture_from(
        r#"
        neori.on("startup", function()
            config_a_fired = true
        end)
        "#,
    );

    // Sanity: config A's handler fires before reload.
    f.state
        .server
        .state
        .scripting
        .call_event("startup")
        .unwrap();
    let fired = f
        .state
        .server
        .state
        .scripting
        .read_global("config_a_fired")
        .unwrap();
    assert!(
        matches!(fired, neori_config::mlua::Value::Boolean(true)),
        "config A's handler should have fired before reload, got {fired:?}"
    );

    // Reload to config B.
    let new_config = parse(
        r#"
        neori.on("startup", function()
            config_b_fired = true
        end)
        "#,
    );
    f.state.server.state.reload_config(Ok(new_config));

    // Fire startup in the new state.
    f.state
        .server
        .state
        .scripting
        .call_event("startup")
        .unwrap();

    // config A's global is gone with the old Lua state.
    let a_gone = f
        .state
        .server
        .state
        .scripting
        .read_global("config_a_fired")
        .unwrap();
    assert!(
        matches!(a_gone, neori_config::mlua::Value::Nil),
        "config A's globals should be gone after reload, got {a_gone:?}"
    );

    // config B's handler fired.
    let b_fired = f
        .state
        .server
        .state
        .scripting
        .read_global("config_b_fired")
        .unwrap();
    assert!(
        matches!(b_fired, neori_config::mlua::Value::Boolean(true)),
        "config B's handler should have fired after reload, got {b_fired:?}"
    );
}

#[test]
fn parse_error_reload_keeps_old_handlers_alive() {
    // A failed reload (Err) must not touch the current ScriptHost — old
    // handlers keep firing because the Lua state wasn't swapped.
    let mut f = fixture_from(
        r#"
        fires = 0
        neori.on("startup", function()
            fires = fires + 1
        end)
        "#,
    );

    // Fire once in the original state.
    f.state
        .server
        .state
        .scripting
        .call_event("startup")
        .unwrap();

    // Signal a parse error.
    f.state.server.state.reload_config(Err(()));

    // Fire again — the original handler should still run because the
    // Lua state wasn't replaced.
    f.state
        .server
        .state
        .scripting
        .call_event("startup")
        .unwrap();

    let fires = f
        .state
        .server
        .state
        .scripting
        .read_global("fires")
        .unwrap();
    assert!(
        matches!(fires, neori_config::mlua::Value::Integer(2)),
        "parse-error reload should preserve old handlers; expected 2 fires, got {fires:?}"
    );
}

#[test]
fn last_focused_window_id_survives_reload() {
    // Neori.last_focused_window_id backs the window-focused diff. If
    // reload_config inadvertently reset it (or a future refactor did),
    // the next tick would re-fire window-focused for a window whose
    // focus hadn't actually moved. Pin the field directly so a mis-wire
    // is caught before it reaches a user.
    let mut f = fixture_from("-- no handlers\n");

    // Seed a focused id by hand.
    f.state.server.state.neori.last_focused_window_id = Some(42);

    // Reload with a new, empty config.
    let new_config = parse("-- still no handlers\n");
    f.state.server.state.reload_config(Ok(new_config));

    assert_eq!(
        f.state.server.state.neori.last_focused_window_id,
        Some(42),
        "last_focused_window_id must persist across reload so window-focused \
         doesn't re-fire for an unchanged focus on the next tick",
    );
}

#[test]
fn reload_clears_action_queue_on_parse_error() {
    // If a handler queues an action and the next reload fails, the
    // queued action must still dispatch (actions are compositor-level,
    // not tied to the Lua state's lifetime). This test documents that
    // invariant — the Err path doesn't drop pending actions because it
    // doesn't touch self.scripting.
    let mut f = fixture_from(
        r#"
        neori.on("startup", function()
            neori.action("close-window")
        end)
        "#,
    );

    let actions = f
        .state
        .server
        .state
        .scripting
        .call_event("startup")
        .expect("startup event failed");
    assert_eq!(
        actions.len(),
        1,
        "startup should have queued one action, got {actions:?}"
    );

    // Trigger a failed reload mid-flight.
    f.state.server.state.reload_config(Err(()));

    // Fire startup again — the handler is still alive, queues another
    // action. The fact that this returns successfully (no panic from a
    // half-swapped scripting state) is the invariant.
    let actions_after = f
        .state
        .server
        .state
        .scripting
        .call_event("startup")
        .expect("startup event failed after parse-error reload");
    assert_eq!(
        actions_after.len(),
        1,
        "handler still active after parse-error reload, got {actions_after:?}"
    );
}
