//! Performance regression tests for Lua event-firing hot paths.
//!
//! These tests exercise the paths that have been the subject of recent perf
//! work: the IPC-driven workspace diff, the per-event cache prime, and the
//! no-handler short-circuit. They're marked `#[ignore]` so they don't run
//! on every `cargo test` (10k-iteration loops take a beat); run explicitly
//! with:
//!
//! ```shell
//! cargo test --release -p neori perf -- --ignored --nocapture
//! ```
//!
//! Each test prints `iterations / total / per-op` so you can eyeball
//! regressions against the last known-good number. The asserts are
//! intentionally loose (~10x headroom) so they don't flake on slow CI
//! machines; the printed numbers are the real signal.

use std::time::Instant;

use neori_config::{Config, WindowSnapshot};

use super::Fixture;

/// Build a fixture with a single Lua snippet. Mirrors `scripting::fixture_from_lua`
/// but lives here so this file is self-contained.
fn fixture(lua: &str) -> Fixture {
    let (config, host) = Config::parse_with_host(std::path::Path::new("perf.lua"), lua)
        .config
        .expect("perf lua snippet failed to parse");
    let mut fixture = Fixture::with_config(config);
    fixture.state.server.state.install_script_host(host);
    fixture
}

fn cheap_window_snapshot() -> WindowSnapshot {
    WindowSnapshot {
        id: 42,
        app_id: Some("perf".to_string()),
        title: Some("perf".to_string()),
        ..Default::default()
    }
}

#[test]
#[ignore = "perf test — run with --ignored --nocapture"]
fn perf_fire_window_event_no_handler() {
    // Baseline: firing an event with no Lua handler registered should cost
    // one HashMap lookup (from event_call) and nothing else. This
    // is the win from skipping all Lua work when nobody's listening.
    let mut f = fixture("-- no handlers registered\n");
    let state = &mut f.state.server.state;
    let snap = cheap_window_snapshot();

    const N: usize = 10_000;
    let start = Instant::now();
    for _ in 0..N {
        state.fire_window_event("window-focused", snap.clone());
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / u32::try_from(N).unwrap();
    println!("fire_window_event no handler: {N} iterations, {elapsed:?}, {per_op:?} per op",);

    // Generous budget — 10k lookups should be well under 10ms even on a slow
    // CI runner. Regressions that reintroduce cache priming here will blow
    // this by orders of magnitude.
    assert!(
        elapsed.as_millis() < 100,
        "fire_window_event with no handler took {elapsed:?} for {N} iterations — \
         likely regression in the event_call short-circuit",
    );
}

#[test]
#[ignore = "perf test — run with --ignored --nocapture"]
fn perf_fire_window_event_with_handler() {
    // Cost ceiling: firing with a registered handler pays the full refresh
    // + call + drain. This isn't a bug to be fixed — it's the baseline for
    // what work the handler path does. Track it so we notice if it grows.
    let mut f = fixture(
        r#"
        received = 0
        neori.on("window-focused", function(w)
            received = received + 1
        end)
        "#,
    );
    let state = &mut f.state.server.state;
    let snap = cheap_window_snapshot();

    const N: usize = 10_000;
    let start = Instant::now();
    for _ in 0..N {
        state.fire_window_event("window-focused", snap.clone());
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / u32::try_from(N).unwrap();
    println!("fire_window_event with handler: {N} iterations, {elapsed:?}, {per_op:?} per op",);

    // Loose 5-second cap — real baseline is on the order of 100ms. If this
    // blows the cap, something major regressed (eager allocations per fire,
    // mlua call overhead, etc.).
    assert!(
        elapsed.as_secs() < 5,
        "fire_window_event with handler took {elapsed:?} for {N} iterations — \
         likely regression in the handler-dispatch path",
    );
}

#[test]
#[ignore = "perf test — run with --ignored --nocapture"]
fn perf_ipc_refresh_layout_steady_state() {
    // ipc_refresh_layout is called from State::refresh every event-loop tick
    // and now also drives Lua workspace events. In the steady state (no
    // actual changes), each call should be near-free — walk workspaces,
    // notice nothing changed, bail.
    let mut f = fixture("-- no workspace handlers\n");
    f.add_output(1, (1920, 1080));
    let state = &mut f.state.server.state;

    // Warm up: let the state settle once so the first-call cost (initial
    // state population) doesn't bias the measurement.
    for _ in 0..10 {
        state.ipc_refresh_layout();
    }

    const N: usize = 10_000;
    let start = Instant::now();
    for _ in 0..N {
        state.ipc_refresh_layout();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / u32::try_from(N).unwrap();
    println!("ipc_refresh_layout steady-state: {N} iterations, {elapsed:?}, {per_op:?} per op",);

    // 10k ticks in steady state should be well under 1 second. The window
    // + workspace walks over ~1 workspace / 0 windows are trivially small.
    assert!(
        elapsed.as_millis() < 1_000,
        "ipc_refresh_layout steady-state took {elapsed:?} for {N} iterations — \
         likely regression in the per-tick walk",
    );
}

#[test]
#[ignore = "perf test — run with --ignored --nocapture"]
fn perf_ipc_refresh_layout_with_lua_handlers() {
    // Same path, but with Lua handlers registered for every workspace /
    // output event. Even with handlers, the per-tick cost should stay
    // bounded because our diff bails on no-change.
    let mut f = fixture(
        r#"
        tick_count = 0
        neori.on("workspace-added", function(_) tick_count = tick_count + 1 end)
        neori.on("workspace-removed", function(_) tick_count = tick_count + 1 end)
        neori.on("workspace-focused", function(_) tick_count = tick_count + 1 end)
        neori.on("output-focused", function(_) tick_count = tick_count + 1 end)
        "#,
    );
    f.add_output(1, (1920, 1080));
    let state = &mut f.state.server.state;

    for _ in 0..10 {
        state.ipc_refresh_layout();
    }

    const N: usize = 10_000;
    let start = Instant::now();
    for _ in 0..N {
        state.ipc_refresh_layout();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / u32::try_from(N).unwrap();
    println!(
        "ipc_refresh_layout with handlers (no change): {N} iterations, {elapsed:?}, {per_op:?} per op",
    );

    // Should match the no-handler path closely: nothing changed, so even
    // with handlers registered, no fire happens → no cache prime, no dispatch.
    assert!(
        elapsed.as_millis() < 1_000,
        "ipc_refresh_layout with handlers (no-change) took {elapsed:?} for {N} iterations — \
         likely regression: diff logic is firing events on no-op ticks",
    );
}
