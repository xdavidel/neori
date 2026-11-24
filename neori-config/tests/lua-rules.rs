//! `neori.rules.add` checks a rule when it's added, so that a typo is an error in the config
//! instead of a rule that never matches. How rules match and apply is tested in the
//! compositor's `src/tests/lua_rules.rs`, which has real windows and outputs.

use neori_config::Config;

#[track_caller]
fn add_error(rule: &str) -> String {
    let code = format!("neori.rules.add {rule}");
    match Config::parse_mem(&code) {
        Ok(_) => panic!("expected an error for {rule}"),
        Err(err) => format!("{err:?}"),
    }
}

#[track_caller]
fn assert_error(rule: &str, expected: &str) {
    let err = add_error(rule);
    assert!(err.contains(expected), "expected {expected:?} in: {err}");
}

#[test]
fn valid_rules_are_accepted() {
    Config::parse_mem(
        r#"
        local id = neori.rules.add {
            where = { app_id = "^firefox$", floating = false },
            where_any = { title = { "a", "b" } },
            where_not = { workspace = 3 },
            where_not_any = { pid = function(pid) return pid == 1 end },
            set = { floating = true, workspace = "web", opacity = 0.9 },
            run = function(w) end,
        }
        assert(id == "1")
        assert(neori.rules.add { id = "pip", set = { floating = true } } == "pip")
        neori.rules.add {
            when = { "window-opened", "window-title-changed" },
            where = { title = "x" },
            set = { opacity = 0.5 },
        }
        neori.rules.add {
            when = "window-opening",
            set = { floating = true, workspace = "web" },
            run = function(w) w:open_on_output("DP-1") end,
        }
        neori.rules.add { when = "monitor-added", where = { name = "^HDMI-" }, set = { scale = 2 } }
        neori.rules.add { when = "workspace-added", where = { idx = 1 }, set = { name = "one" } }
        neori.rules.add { when = "device-added", where = { touchpad = true }, set = { tap = true } }
        neori.rules.add { when = "layer-added", where = { namespace = "waybar" }, run = print }
        assert(neori.rules.remove("pip"))
        assert(not neori.rules.remove("pip"))
        assert(neori.rules.remove(id))
        "#,
    )
    .unwrap();
}

#[test]
fn a_rule_needs_set_or_run() {
    assert_error(
        "{ where = { app_id = 'x' } }",
        "a rule needs `set`, `run` or both",
    );
}

#[test]
fn unknown_keys_are_errors() {
    assert_error(
        "{ were = { app_id = 'x' }, run = print }",
        "unknown rule key 'were'",
    );
    assert_error(
        "{ where = { appid = 'x' }, run = print }",
        "where: Window has no property 'appid' to match",
    );
    assert_error(
        "{ set = { floting = true } }",
        "set: Window has no property 'floting' to set",
    );
    // Properties are checked against the object of the event.
    assert_error(
        "{ when = 'monitor-added', where = { app_id = 'x' }, run = print }",
        "where: Output has no property 'app_id' to match",
    );
    assert_error(
        "{ when = 'layer-added', set = { opacity = 1 } }",
        "set: LayerSurface has no property 'opacity' to set",
    );
}

#[test]
fn values_are_checked() {
    assert_error(
        "{ where = { title = '(' }, run = print }",
        "where.title: invalid regex '('",
    );
    assert_error(
        "{ where = { title = {} }, run = print }",
        "where.title: a list of alternatives can't be empty",
    );
    assert_error(
        "{ where = { title = { { 'a' } } }, run = print }",
        "alternatives can't be lists themselves",
    );
    assert_error("{ run = 1 }", "`run` must be a function");
    assert_error("{ id = 1, run = print }", "`id` must be a string");
    assert_error("{ set = 1 }", "`set` must be a table");
}

#[test]
fn events_are_checked() {
    assert_error(
        "{ when = 'window-opend', run = print }",
        "unknown event 'window-opend'",
    );
    assert_error(
        "{ when = 'startup', run = print }",
        "the event 'startup' doesn't pass an object to match",
    );
    assert_error("{ when = {}, run = print }", "`when` must be an event name");
}

#[test]
fn window_opening_only_sets_what_works_before_the_window_exists() {
    assert_error(
        "{ when = 'window-opening', set = { opacity = 0.5 } }",
        "set: opacity = 0.5 can't be set at window-opening",
    );
    // open_on_workspace takes only a name.
    assert_error(
        "{ when = 'window-opening', set = { workspace = 2 } }",
        "set: workspace = 2 can't be set at window-opening",
    );
}
