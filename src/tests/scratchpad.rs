//! Tests for the scratchpad spawn wiring: toggling an empty scratchpad launches its
//! configured command, and the window that appears afterwards is adopted into it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use neori_config::{Action, Config};

use super::*;

/// A unique marker file per test, cleaned up on creation so a stale file can't mask a
/// missing spawn.
fn marker(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("neori-scratchpad-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// Spawns run on a background thread (and double-fork), so poll for the marker.
fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn toggle_empty_scratchpad_spawns_configured_command() {
    let spawned = marker("default");
    let config = Config::parse_mem(&format!(
        r#"neori.scratchpad {{ spawn = {{ "touch", "{}" }} }}"#,
        spawned.display()
    ))
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    // Toggling the empty scratchpad marks a spawn as pending and runs the command.
    f.neori_state()
        .do_action(Action::ToggleScratchpad { name: None }, false);
    assert!(f.neori().layout.has_pending_scratchpad_spawn(None));
    assert!(
        wait_for_file(&spawned, Duration::from_secs(5)),
        "configured spawn command did not run"
    );

    // The scratchpad is still empty (no window adopted), but the pending spawn blocks a
    // second launch until a window shows up.
    let _ = std::fs::remove_file(&spawned);
    f.neori_state()
        .do_action(Action::ToggleScratchpad { name: None }, false);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !spawned.exists(),
        "second toggle respawned the command while a spawn was pending"
    );
    assert!(f.neori().layout.has_pending_scratchpad_spawn(None));
}

#[test]
fn toggle_named_scratchpad_spawns_its_own_command() {
    let spawned = marker("term");
    let config = Config::parse_mem(&format!(
        r#"neori.scratchpad("term", {{ spawn_sh = "touch {}" }})"#,
        spawned.display()
    ))
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    // The named scratchpad looks up its own command.
    f.neori_state().do_action(
        Action::ToggleScratchpad {
            name: Some("term".to_string()),
        },
        false,
    );
    assert!(f.neori().layout.has_pending_scratchpad_spawn(Some("term")));
    assert!(
        wait_for_file(&spawned, Duration::from_secs(5)),
        "named scratchpad spawn command did not run"
    );

    // The default scratchpad has no configured command: no pending spawn for it.
    f.neori_state()
        .do_action(Action::ToggleScratchpad { name: None }, false);
    assert!(!f.neori().layout.has_pending_scratchpad_spawn(None));
    assert!(f.neori().layout.has_pending_scratchpad_spawn(Some("term")));
}

#[test]
fn toggle_scratchpad_without_spawn_command_sets_no_pending() {
    // A declared scratchpad with no `spawn`/`spawn_sh` is only ever filled by sending a
    // window to it, so toggling must not mark a spawn as pending.
    let config = Config::parse_mem(r#"neori.scratchpad("bare")"#).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    f.neori_state().do_action(
        Action::ToggleScratchpad {
            name: Some("bare".to_string()),
        },
        false,
    );
    assert!(!f.neori().layout.has_pending_scratchpad_spawn(Some("bare")));

    // Same for a scratchpad that was never declared at all.
    f.neori_state()
        .do_action(Action::ToggleScratchpad { name: None }, false);
    assert!(!f.neori().layout.has_pending_scratchpad_spawn(None));
}

#[test]
fn window_mapped_after_toggle_is_adopted_into_scratchpad() {
    let spawned = marker("adopt");
    let config = Config::parse_mem(&format!(
        r#"neori.scratchpad {{ spawn = {{ "touch", "{}" }} }}"#,
        spawned.display()
    ))
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    // Toggle to arm the pending spawn.
    f.neori_state()
        .do_action(Action::ToggleScratchpad { name: None }, false);
    assert!(f.neori().layout.has_pending_scratchpad_spawn(None));

    // Map the next window; the compositor must adopt it into the waiting scratchpad
    // instead of leaving it on the workspace.
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
        !f.neori().layout.has_pending_scratchpad_spawn(None),
        "adoption should consume the pending spawn"
    );
    let window = f
        .neori()
        .layout
        .focus()
        .expect("the adopted window should be shown and focused")
        .window
        .clone();

    // The window now belongs to the scratchpad: toggling hides it from the workspace.
    assert!(!f.neori().layout.toggle_scratchpad(None));
    assert!(!f
        .neori()
        .layout
        .workspaces()
        .any(|(_, _, ws)| ws.has_window(&window)));

    let _ = std::fs::remove_file(&spawned);
}
