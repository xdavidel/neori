use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use neori_config::Config;
use neori_ipc::{Action, Reply, Request, Response, WorkspaceReferenceArg};

use super::fixture::Fixture;

static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(0);

fn start_ipc(fixture: &mut Fixture) -> PathBuf {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let name = format!("neori-test-{}-{id}", std::process::id());
    let handle = fixture.state.server.state.neori.event_loop.clone();
    let server = fixture
        .state
        .server
        .state
        .neori
        .ipc_server
        .as_mut()
        .unwrap();
    server
        .start_socket(&handle, Some(std::ffi::OsStr::new(&name)))
        .unwrap();
    server.socket_path.clone().unwrap()
}

fn send_action(fixture: &mut Fixture, path: &Path, action: Action) -> Reply {
    let path = path.to_owned();
    let client = std::thread::spawn(move || {
        let stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();

        let mut buf = serde_json::to_string(&Request::Action(action)).unwrap();
        buf.push('\n');
        stream
            .try_clone()
            .unwrap()
            .write_all(buf.as_bytes())
            .unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str::<Reply>(&line).unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.is_finished() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the IPC reply"
        );
        fixture.dispatch();
        std::thread::sleep(Duration::from_millis(1));
    }

    client.join().unwrap()
}

#[test]
fn ipc_set_shake_to_find_applies_at_runtime() {
    let mut fixture = Fixture::new();
    let path = start_ipc(&mut fixture);

    let reply = send_action(
        &mut fixture,
        &path,
        Action::SetShakeToFind {
            settings: neori_ipc::ShakeToFindToSet {
                enable: Some(true),
                window_ms: Some(800),
                min_reversals: Some(2),
                min_step: Some(6.5),
                reversal_angle_deg: Some(135.),
                ..Default::default()
            },
        },
    );
    assert!(
        matches!(reply, Ok(Response::Handled)),
        "unexpected reply: {reply:?}"
    );

    {
        let config = fixture.state.server.state.neori.config.borrow();
        let shake = config.cursor.shake_to_find;
        assert!(shake.enable);
        assert_eq!(shake.window_ms, 800);
        assert_eq!(shake.min_reversals, 2);
        assert_eq!(shake.min_step, 6.5);
        assert_eq!(shake.reversal_angle_deg, 135.);
        // Unset fields keep their defaults.
        assert_eq!(shake.size_multiplier, 4.);
        assert_eq!(shake.timeout_ms, 1000);
    }

    // The live CursorShake must have picked up enable = true, otherwise triggering it does
    // nothing.
    let reply = send_action(&mut fixture, &path, Action::EnlargeCursor {});
    assert!(
        matches!(reply, Ok(Response::Handled)),
        "unexpected reply: {reply:?}"
    );
    assert!(fixture.neori().cursor_shake.are_animations_ongoing());

    std::fs::remove_file(&path).ok();
}

#[test]
fn ipc_set_shake_to_find_rejects_invalid_settings() {
    let mut fixture = Fixture::new();
    let path = start_ipc(&mut fixture);

    let reply = send_action(
        &mut fixture,
        &path,
        Action::SetShakeToFind {
            settings: neori_ipc::ShakeToFindToSet {
                min_reversals: Some(0),
                ..Default::default()
            },
        },
    );

    match reply {
        Err(err) => assert!(err.contains("min_reversals"), "unexpected error: {err}"),
        Ok(reply) => panic!("expected an error, got {reply:?}"),
    }

    std::fs::remove_file(&path).ok();
}

#[test]
fn ipc_general_actions_are_dispatched() {
    let mut fixture = Fixture::new();
    fixture.add_output(1, (1920, 1080));
    fixture.neori_focus_output(1);
    let path = start_ipc(&mut fixture);

    // Empty-struct variant.
    let reply = send_action(&mut fixture, &path, Action::FocusColumnLeft {});
    assert!(
        matches!(reply, Ok(Response::Handled)),
        "unexpected reply: {reply:?}"
    );

    // Struct variant with an optional field, naming the focused workspace.
    let reply = send_action(
        &mut fixture,
        &path,
        Action::SetWorkspaceName {
            name: String::from("ipc-general"),
            workspace: None,
        },
    );
    assert!(
        matches!(reply, Ok(Response::Handled)),
        "unexpected reply: {reply:?}"
    );
    assert_eq!(
        fixture.neori().layout.active_workspace().unwrap().name(),
        Some(&String::from("ipc-general")),
    );

    // Variant whose config-only ByRef counterpart was folded into it.
    let reply = send_action(
        &mut fixture,
        &path,
        Action::MoveWorkspaceToIndex {
            index: 1,
            reference: Some(WorkspaceReferenceArg::Name(String::from("ipc-general"))),
        },
    );
    assert!(
        matches!(reply, Ok(Response::Handled)),
        "unexpected reply: {reply:?}"
    );

    std::fs::remove_file(&path).ok();
}

#[test]
fn ipc_dispatches_actions_parsed_from_lua_config() {
    let config = Config::parse_mem(
        r#"
        neori.bind("Mod+1", { focus_workspace = 1 })
        neori.bind("Mod+2", "move-column-left")
        neori.bind("Mod+3", { toggle_overview = true })
        neori.bind("Mod+4", { close_window = true })
        neori.bind("Mod+5", { move_column_to_workspace = 2 })
        neori.bind("Mod+6", { set_column_width = "+10%" })
        "#,
    )
    .unwrap();

    let mut fixture = Fixture::new();
    fixture.add_output(1, (1920, 1080));
    fixture.neori_focus_output(1);
    let path = start_ipc(&mut fixture);

    // The Lua parser produces the same Action type that IPC uses, so the actions from a
    // config can be sent over IPC directly.
    for bind in &config.binds.0 {
        let reply = send_action(&mut fixture, &path, bind.action.clone());
        assert!(
            matches!(reply, Ok(Response::Handled)),
            "unexpected reply for {:?}: {reply:?}",
            bind.action,
        );
    }

    // At least one of the actions must have an observable effect.
    assert!(fixture.neori().layout.is_overview_open());

    std::fs::remove_file(&path).ok();
}
