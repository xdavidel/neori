use neori_config::Config;
use smithay::backend::input::InputTime;

use super::*;

/// Parse a Lua config snippet into a [`Config`]; panics on parse errors so tests surface them
/// clearly instead of silently falling back to defaults.
fn config_from_lua(snippet: &str) -> Config {
    Config::parse_mem(snippet).expect("lua snippet failed to parse")
}

#[test]
fn lua_config_create_virtual_creates_output_at_startup() {
    let config = config_from_lua(
        r#"
        neori.output("sunshine", {
            create_virtual = true,
            mode = "1920x1080@144.000",
        })
        "#,
    );

    let mut f = Fixture::with_config(config);

    // State::new runs `create_virtual_outputs_from_config`, so the output should already exist.
    let output = f
        .neori()
        .global_space
        .outputs()
        .find(|o| o.name() == "sunshine")
        .expect("virtual output 'sunshine' should have been created at startup")
        .clone();

    let modes = output.modes();
    assert_eq!(modes.len(), 1);
    assert_eq!(modes[0].size.w, 1920);
    assert_eq!(modes[0].size.h, 1080);
    // The Lua mode string is "1920x1080@144.000"; stored as millihertz.
    assert_eq!(modes[0].refresh, 144_000);
}

#[test]
fn lua_config_reload_creates_newly_declared_virtual_output() {
    let mut f = Fixture::new();

    // No output yet.
    assert!(f
        .neori()
        .global_space
        .outputs()
        .all(|o| o.name() != "sunshine"));

    // Simulate a live config reload that now declares a create_virtual output.
    let new_config = config_from_lua(
        r#"neori.output("sunshine", { create_virtual = true })"#,
    );
    let state = f.neori_state();
    state.neori.config.replace(new_config);
    state.reload_output_config();

    let found = state
        .neori
        .global_space
        .outputs()
        .any(|o| o.name() == "sunshine");
    assert!(found, "reload_output_config should have created the virtual output");
}

#[test]
fn creating_virtual_output_fires_monitor_added_event() {
    // Register a monitor-added handler that creates a second virtual output named "canary".
    // If monitor-added fires for a virtual output just like it does for physical ones, the
    // handler's action will run on the next dispatch and "canary" will appear.
    let config = config_from_lua(
        r#"
        neori.on("monitor-added", neori.create_virtual_output { name = "canary" })
        "#,
    );
    let mut f = Fixture::with_config(config);

    // Trigger creation of a virtual output; `fire_monitor_event` is scheduled via insert_idle
    // on the neori event loop.
    f.neori_state().do_action(
        neori_ipc::Action::CreateVirtualOutput {
            name: Some("sunshine".to_string()),
            width: None,
            height: None,
            refresh_rate: None,
        },
        false, InputTime::now(),
    );

    // Drain the idle queue so the scheduled fire_monitor_event runs. The action it fires
    // creates "canary" which itself schedules another monitor-added, so a second pass drains
    // that one (and verifies it also fires correctly).
    f.state.server.dispatch();
    f.state.server.dispatch();

    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "sunshine"),
        "virtual output 'sunshine' should exist"
    );
    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "canary"),
        "monitor-added should have fired and created 'canary'"
    );
}

#[test]
fn removing_virtual_output_fires_monitor_removed_event() {
    // monitor-removed handler creates an "epitaph" virtual output as an observable side-effect.
    let config = config_from_lua(
        r#"
        neori.on("monitor-removed", neori.create_virtual_output { name = "epitaph" })
        "#,
    );
    let mut f = Fixture::with_config(config);

    f.neori_state().do_action(
        neori_ipc::Action::CreateVirtualOutput {
            name: Some("doomed".to_string()),
            width: None,
            height: None,
            refresh_rate: None,
        },
        false, InputTime::now(),
    );
    // Drain the monitor-added firing for "doomed" before testing removal.
    f.state.server.dispatch();

    f.neori_state().do_action(
        neori_ipc::Action::RemoveVirtualOutput {
            name: "doomed".to_string(),
        },
        false, InputTime::now(),
    );
    f.state.server.dispatch();
    // The monitor-removed handler schedules another add; drain that too.
    f.state.server.dispatch();

    assert!(
        !f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "doomed"),
        "virtual output 'doomed' should have been removed"
    );
    assert!(
        f.neori()
            .global_space
            .outputs()
            .any(|o| o.name() == "epitaph"),
        "monitor-removed should have fired and created 'epitaph'"
    );
}

#[test]
fn lua_action_create_virtual_output_dispatches() {
    let mut f = Fixture::new();

    // `neori.create_virtual_output { ... }` is dispatched exactly like calling
    // `backend.create_virtual_output`, so firing the Action should produce a real output.
    f.neori_state().do_action(
        neori_ipc::Action::CreateVirtualOutput {
            name: Some("from-lua".to_string()),
            width: Some(1920),
            height: Some(1080),
            refresh_rate: Some(60),
        },
        false, InputTime::now(),
    );

    let created = f
        .neori()
        .global_space
        .outputs()
        .any(|o| o.name() == "from-lua");
    assert!(
        created,
        "Action::CreateVirtualOutput should have created the output"
    );

    // And removing it via Action::RemoveVirtualOutput should drop it from the compositor.
    f.neori_state().do_action(
        neori_ipc::Action::RemoveVirtualOutput {
            name: "from-lua".to_string(),
        },
        false, InputTime::now(),
    );

    let still_there = f
        .neori()
        .global_space
        .outputs()
        .any(|o| o.name() == "from-lua");
    assert!(
        !still_there,
        "Action::RemoveVirtualOutput should have removed the output"
    );
}

#[test]
fn lua_config_create_virtual_kebab_alias_is_accepted() {
    // Both snake_case and kebab-case keys are accepted by the Lua parser.
    let config = config_from_lua(
        r#"neori.output { name = "HEADLESS-1", ["create-virtual"] = true }"#,
    );
    let mut f = Fixture::with_config(config);

    let exists = f
        .neori()
        .global_space
        .outputs()
        .any(|o| o.name() == "HEADLESS-1");
    assert!(
        exists,
        "virtual output 'HEADLESS-1' from kebab-case create-virtual should have been created"
    );
}

#[test]
fn virtual_output_name_charset_is_restricted() {
    // Mirrors Umbriel: ASCII letters, digits, '-', '_', '.' only.
    let mut f = Fixture::new();
    let bad_names = [
        ("empty string", ""),
        ("whitespace", "  "),
        ("space inside", "my output"),
        ("slash", "hdmi/1"),
        ("non-ascii", "ドック"),
        ("newline", "a\nb"),
    ];
    for (why, name) in bad_names {
        let state = f.neori_state();
        let result =
            state
                .backend
                .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some(name.to_owned()));
        assert!(
            result.is_err(),
            "{why}: expected validation failure for name {name:?}, got {result:?}"
        );
    }

    // Good names should still work.
    let good_names = ["sunshine", "HEADLESS-2", "cast_1", "remote.desk.1"];
    for name in good_names {
        let state = f.neori_state();
        let result =
            state
                .backend
                .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some(name.to_owned()));
        assert!(result.is_ok(), "expected {name:?} to be accepted, got {result:?}");
    }
}

#[test]
fn virtual_output_name_uniqueness_is_case_insensitive() {
    let mut f = Fixture::new();
    {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some("Sunshine".to_owned()))
            .unwrap();
    }
    let state = f.neori_state();
    let err = state
        .backend
        .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some("sunshine".to_owned()))
        .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected case-insensitive collision error, got {err:?}"
    );
}

#[test]
fn virtual_output_custom_mode_does_not_accumulate_modes() {
    let mut f = Fixture::new();

    let name = {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(
                &mut state.neori,
                1920,
                1080,
                60,
                Some("sunshine".to_owned()),
            )
            .unwrap()
    };

    let output = f
        .neori()
        .global_space
        .outputs()
        .find(|o| o.name() == name)
        .unwrap()
        .clone();

    // Sanity: single initial mode.
    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 1920);
        assert_eq!(modes[0].size.h, 1080);
    }

    // 1080p -> 3200x1800
    {
        let state = f.neori_state();
        state.apply_transient_output_config(
            &name,
            neori_ipc::OutputAction::CustomMode {
                mode: neori_ipc::ConfiguredMode {
                    width: 3200,
                    height: 1800,
                    refresh: Some(60.0),
                },
            },
        );
    }

    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 3200);
        assert_eq!(modes[0].size.h, 1800);
    }

    // 3200x1800 -> 1080p
    {
        let state = f.neori_state();
        state.apply_transient_output_config(
            &name,
            neori_ipc::OutputAction::CustomMode {
                mode: neori_ipc::ConfiguredMode {
                    width: 1920,
                    height: 1080,
                    refresh: Some(60.0),
                },
            },
        );
    }

    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 1920);
        assert_eq!(modes[0].size.h, 1080);
    }
}

#[test]
fn removing_off_virtual_output_does_not_panic() {
    let mut f = Fixture::new();

    let name = {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some("virt".to_owned()))
            .unwrap()
    };

    {
        let state = f.neori_state();
        state.modify_output_config(&name, |config| {
            config.off = true;
        });
    }

    {
        let state = f.neori_state();
        state
            .backend
            .remove_virtual_output(&mut state.neori, &name)
            .unwrap();
    }
}

#[test]
fn config_remove_virtual_output_fails_while_configured() {
    let mut f = Fixture::new();
    let name = "virt";

    {
        let state = f.neori_state();
        state.modify_output_config(name, |config| {
            config.create_virtual = true;
        });
    }

    let result = {
        let state = f.neori_state();
        state.backend.remove_virtual_output(&mut state.neori, name)
    };

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("is configured"));
}

#[test]
fn unnamed_virtual_outputs_do_not_advance_counter_for_named_outputs() {
    let mut f = Fixture::new();

    {
        let state = f.neori_state();
        state.modify_output_config("virt", |_| {});
    }

    let headless_1 = {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(&mut state.neori, 1920, 1080, 60, None)
            .unwrap()
    };

    assert_eq!(headless_1, "HEADLESS-1");

    let headless_2 = {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(&mut state.neori, 1920, 1080, 60, None)
            .unwrap()
    };

    assert_eq!(headless_2, "HEADLESS-2");
}

#[test]
fn touch_input_targets_virtual_output_when_focused() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let name = {
        let state = f.neori_state();
        state
            .backend
            .create_virtual_output(&mut state.neori, 1920, 1080, 60, Some("virt".to_owned()))
            .unwrap()
    };

    let virt = f
        .neori()
        .global_space
        .outputs()
        .find(|o| o.name() == name)
        .unwrap()
        .clone();

    f.neori().layout.focus_output(&virt);

    // With no explicit `input.touch.map-to-output` configured,
    // touch should follow the active output (which may be virtual).
    let touch_output = f.neori().output_for_touch().unwrap().clone();
    assert_eq!(touch_output, virt);
}
