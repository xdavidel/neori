//! Tests for `neori.rules`: rules matching real windows, outputs and workspaces when events
//! fire. The checks done when a rule is added are tested in `neori-config/tests/lua-rules.rs`.

use neori_config::Config;
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;

use super::client::ClientId;
use super::*;

fn fixture_from_lua(snippet: &str) -> Fixture {
    let (config, host) = Config::parse_with_host(std::path::Path::new("test.lua"), snippet)
        .config
        .expect("lua snippet failed to parse");
    let mut fixture = Fixture::with_config(config);
    fixture.state.server.state.install_script_host(host);
    fixture
}

/// Opens a window with this app id and title, and runs the event loop until the per-refresh
/// property diff ran.
fn open_window(f: &mut Fixture, id: ClientId, app_id: &str, title: &str) -> ClientWlSurface {
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id(app_id.to_owned());
    window.set_title(title);
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    refresh(f);
    surface
}

fn refresh(f: &mut Fixture) {
    f.state.server.dispatch();
    f.state.server.dispatch();
}

/// Evaluates Lua in the running compositor and returns its values as strings.
#[track_caller]
fn eval(f: &mut Fixture, code: &str) -> Vec<String> {
    match f.state.server.state.eval_lua(code) {
        Ok(output) => output.values,
        Err(err) => panic!("{code}: {err}"),
    }
}

/// `expr` evaluated for the window with this app id.
#[track_caller]
fn of_window(f: &mut Fixture, app_id: &str, expr: &str) -> String {
    let code = format!(
        "for _, w in ipairs(neori.windows()) do \
             if w:app_id() == {app_id:?} then return {expr} end \
         end \
         error('no window {app_id}')"
    );
    eval(f, &code).join(", ")
}

fn global(f: &mut Fixture, name: &str) -> String {
    eval(f, &format!("return tostring({name})")).join(", ")
}

#[test]
fn a_window_rule_sets_properties_and_runs_once_the_window_opened() {
    let mut f = fixture_from_lua(
        r#"
        ran = ""
        neori.rules.add {
            where = { app_id = "^pip$" },
            set = { floating = true, opacity = 0.5 },
            run = function(w) ran = ran .. w:app_id() .. ":" .. tostring(w:is_floating()) .. ";" end,
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "pip", "Picture");
    open_window(&mut f, id, "pipe", "Other");

    assert_eq!(of_window(&mut f, "pip", "w:is_floating()"), "true");
    assert_eq!(of_window(&mut f, "pip", "w:opacity()"), "0.5");
    // The regex is anchored, so "pipe" doesn't match.
    assert_eq!(of_window(&mut f, "pipe", "w:is_floating()"), "false");
    assert_eq!(of_window(&mut f, "pipe", "w:opacity()"), "1.0");
    // `run` ran once, after `set`.
    assert_eq!(global(&mut f, "ran"), "pip:true;");
}

#[test]
fn a_window_rule_opens_the_window_where_it_should_before_its_first_frame() {
    // Without `when`, the keys with an open_* method apply at window-opening: the window is
    // floating and fullscreen in its first configure, rather than moved there after mapping.
    let mut f = fixture_from_lua(
        r#"
        neori.rules.add {
            where = { app_id = "^right$" },
            set = { output = "headless-2", fullscreen = true },
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    f.neori_focus_output(1);

    let id = f.add_client();
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id("right".to_owned());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    // The first configure already has the fullscreen size of the second output.
    let window = f.client(id).window(&surface);
    assert_eq!(
        window.configures_received.last().unwrap().1.size,
        (1280, 720)
    );
    window.attach_new_buffer();
    window.set_size(1280, 720);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    refresh(&mut f);

    assert_eq!(of_window(&mut f, "right", "w:output()"), "headless-2");
    assert_eq!(of_window(&mut f, "right", "w:is_fullscreen()"), "true");
}

#[test]
fn a_window_rule_sends_to_a_workspace_by_index_once_opened() {
    // open_on_workspace takes only names, so an index applies once the window opened.
    let mut f = fixture_from_lua(
        r#"neori.rules.add { where = { app_id = "^two$" }, set = { workspace = 2 } }"#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "one", "");
    open_window(&mut f, id, "two", "");

    assert_eq!(of_window(&mut f, "two", "w:workspace().idx"), "2");
    assert_eq!(of_window(&mut f, "one", "w:workspace().idx"), "1");
}

#[test]
fn conditions_combine() {
    let mut f = fixture_from_lua(
        r#"
        hits = ""
        local function hit(name)
            return function(w) hits = hits .. name .. ":" .. w:app_id() .. ";" end
        end
        -- Every key must match; a list is alternatives.
        neori.rules.add { where = { app_id = { "^a$", "^b$" }, title = "x" }, run = hit("where") }
        -- Any key.
        neori.rules.add { where_any = { app_id = "^a$", title = "^y$" }, run = hit("any") }
        -- Skipped only when every key matches.
        neori.rules.add { where_not = { app_id = "^a$", title = "x" }, run = hit("not") }
        -- Skipped when any key matches.
        neori.rules.add { where_not_any = { app_id = "^a$", title = "x" }, run = hit("not_any") }
        -- A function gets the value and the window.
        neori.rules.add {
            where = { title = function(title, w) return #title == 1 and w:app_id() ~= "a" end },
            run = hit("fn"),
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "a", "x");
    open_window(&mut f, id, "b", "y");
    open_window(&mut f, id, "c", "xx");

    assert_eq!(
        global(&mut f, "hits"),
        // a/x: where, any. not and not_any skip it.
        "where:a;any:a;\
         any:b;not:b;not_any:b;fn:b;\
         not:c;"
    );
}

#[test]
fn rules_with_when_apply_on_those_events() {
    // A title that arrives after the window opened.
    let mut f = fixture_from_lua(
        r#"
        neori.rules.add {
            when = { "window-opened", "window-title-changed" },
            where = { title = "^Picture-in-Picture$" },
            set = { floating = true },
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = open_window(&mut f, id, "firefox", "Loading");
    assert_eq!(of_window(&mut f, "firefox", "w:is_floating()"), "false");

    let window = f.client(id).window(&surface);
    window.set_title("Picture-in-Picture");
    window.commit();
    f.double_roundtrip(id);
    refresh(&mut f);
    assert_eq!(of_window(&mut f, "firefox", "w:is_floating()"), "true");
}

#[test]
fn window_opening_in_when_runs_before_the_window_opens() {
    let mut f = fixture_from_lua(
        r#"
        neori.rules.add {
            when = "window-opening",
            where = { app_id = "^float$" },
            set = { floating = true },
            -- The window has no id yet.
            run = function(w) run_with_id = w:id() end,
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "float", "");
    assert_eq!(of_window(&mut f, "float", "w:is_floating()"), "true");
    assert_eq!(global(&mut f, "run_with_id"), "0");
}

#[test]
fn removed_rules_stop_applying() {
    let mut f = fixture_from_lua(
        r#"
        count = 0
        local rule = { run = function() count = count + 1 end }
        neori.rules.add(rule)
        neori.rules.add { id = "named", run = function() count = count + 10 end }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "a", "");
    assert_eq!(global(&mut f, "count"), "11");

    assert_eq!(eval(&mut f, "return neori.rules.remove('named')"), ["true"]);
    open_window(&mut f, id, "b", "");
    assert_eq!(global(&mut f, "count"), "12");
}

#[test]
fn a_failing_rule_doesnt_stop_the_others() {
    let mut f = fixture_from_lua(
        r#"
        ran = false
        neori.rules.add { run = function() error("broken") end }
        neori.rules.add { run = function() ran = true end }
        "#,
    );
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "a", "");
    assert_eq!(global(&mut f, "ran"), "true");
}

#[test]
fn output_rules_apply_when_a_monitor_is_added() {
    let mut f = fixture_from_lua(
        r#"
        neori.rules.add {
            when = "monitor-added",
            where = { name = "^headless-2$" },
            set = { scale = 2 },
        }
        "#,
    );
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    // monitor-added fires from an idle callback.
    refresh(&mut f);

    let scale = |f: &mut Fixture, name: &str| {
        f.neori()
            .global_space
            .outputs()
            .find(|o| o.name() == name)
            .unwrap()
            .current_scale()
            .fractional_scale()
    };
    assert_eq!(scale(&mut f, "headless-1"), 1.);
    assert_eq!(scale(&mut f, "headless-2"), 2.);
}

#[test]
fn matches_and_apply_work_on_any_object() {
    let mut f = fixture_from_lua("");
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    open_window(&mut f, id, "a", "title");

    assert_eq!(
        eval(
            &mut f,
            r#"
            local w = neori.windows()[1]
            local out = neori.outputs()[1]
            local ws = neori.workspaces()[1]
            return neori.rules.matches(w, { where = { app_id = "a", focused = true } }),
                neori.rules.matches(w, { where = { title = "^x" } }),
                neori.rules.matches(out, { where = { name = "headless" } }),
                neori.rules.matches(ws, { where = { idx = 1, focused = true } }),
                neori.rules.apply(w, { where = { title = "title" }, set = { floating = true } }),
                w:is_floating()
            "#,
        ),
        ["true", "false", "true", "true", "true", "true"]
    );

    // Properties are checked against the object's class.
    let err = f
        .state
        .server
        .state
        .eval_lua("return neori.rules.matches(neori.outputs()[1], { where = { app_id = 'a' } })")
        .unwrap_err();
    assert!(err.contains("Output has no property 'app_id'"), "{err}");
}
