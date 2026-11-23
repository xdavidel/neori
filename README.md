<h1 align="center">neori</h1>

<p align="center">
    <a href="https://www.gnu.org/licenses/gpl-3.0.html"><img alt="License: GPL-3.0-or-later" src="https://img.shields.io/badge/License-GPLv3%2B-blue.svg"></a>
</p>

Neori is a **scrollable-tiling Wayland compositor** — a fork of [niri](https://github.com/niri-wm/niri) with extended configuration support.

Windows are arranged in columns on an infinite horizontal strip. Opening a new window never causes existing windows to resize. Scroll left and right through your workspace.

### Neori-specific features

- **Lua-based configuration** — configure with `config.lua` instead of KDL
- **Window-rule extensions** — `geometry_corner_radius`, `clip_to_geometry`, `draw_border_with_background`, `opacity`, `tiled_state`, `baba_is_float`, `variable_refresh_rate` and more
- **Scratchpads** — named scratchpads (plus a default one): hide a window with `send-to-scratchpad term`, bring it back with `remove-from-scratchpad term`, or toggle it with `toggle-scratchpad term`; declare each with `neori.scratchpad("term", { spawn = { "kitty" } })` so toggling an empty scratchpad spawns its window
- **Keycode-based binds** — binds match the physical key by default, so shortcuts keep working across keyboard layouts; opt out with `keycode = false`
- **Shake-to-find cursor** — shake the mouse to temporarily enlarge the cursor, with configurable size, timing, and detection sensitivity (`neori.cursor.shake_to_find`); trigger it with the `enlarge-cursor` action or retune it at runtime with `neori msg action set-shake-to-find`
- **Lid and tablet-mode events** — `neori.on("lid-open" / "lid-closed" / "tablet-mode-on" / "tablet-mode-off", ...)` handlers, run even while the session is locked
- **Smarter workspace navigation** — sparse on-demand numbering (no auto-created gaps), navigation skips empty workspaces, and at most one empty workspace is kept
- **Cursor warp on workspace switch** — moving focus to a workspace on another monitor moves the pointer with it
- **Extended IPC state** — `neori msg windows` exposes `is_fullscreen`, `is_window_cast_target`, `is_active_in_column`, and `is_xwayland`
- **NIRI_SOCKET compatibility** — IPC remains niri-compatible (`niri msg ...` works, DMS can detect via `NIRI_SOCKET`)
- **Config validation and API** — full `neori.*` Lua API with `include`/strict mode, `startup`/`monitor-added`/`monitor-removed` hooks, and switch events
- **Live Lua scripting** — `Window`, `Output`, `Workspace` and `Device` objects that always read current state and apply changes right away, signals (`neori.connect`, per-object `w:connect("closed", …)`, custom signals with `neori.emit`, property signals like `window-title-changed`, listing with `neori.signals()`), timers (`neori.after`, `neori.every`), `neori msg lua` to run Lua from a shell, and type definitions for lua-language-server

### Neori features (inherited)

- Scrollable-tiling layout
- Dynamic workspaces (per-monitor)
- Floating windows, fullscreen, maximized
- Window tabs
- Touchpad and mouse gestures
- Gradient borders with Oklab/Oklch support
- Background blur
- Animations with custom shader support
- Screencasting via xdg-desktop-portal-gnome
- Xwayland via xwayland-satellite
- Fractional scaling (pixel-perfect)
- HiDPI support
- Multi-GPU support
- VRR
- Spill (visual overflow on the left side)
- Block-out from screencasts
- Dynamic screencast target
- Screen reader support

### Getting started

Follow the [neori getting started guide](https://neori-wm.github.io/neori/Getting-Started.html) for build/runtime details. The main difference is configuration: use `~/.config/neori/config.lua` (or `/etc/neori/config.lua`, or `NEORI_CONFIG`). See `resources/default-config.lua` for a reference.

### Development: testing in a nested session

The easiest way to try a local build is to run it as a nested window inside your current Wayland or X11 session. Neori automatically picks the winit backend when `WAYLAND_DISPLAY`, `WAYLAND_SOCKET`, or `DISPLAY` is set, so do **not** pass `--session`.

```bash
# From a terminal in your existing session:
nix develop --command cargo run --release

# Or use a throwaway config instead of ~/.config/neori/config.lua:
nix develop --command cargo run --release -- -c /tmp/neori-test.lua
```

Inside the nested session, `Mod` is **Alt** by default. The compositor logs the IPC socket path at startup:

```
IPC listening on: /run/user/1000/neori.neori-XXXX.12345.sock
```

Processes started inside the nested session inherit `NEORI_SOCKET`. From a host terminal, export it yourself:

```bash
export NEORI_SOCKET=/run/user/1000/neori.neori-XXXX.12345.sock

target/release/neori msg action set-shake-to-find --enable true --size-multiplier 6 --window-ms 500
target/release/neori msg action enlarge-cursor
target/release/neori msg action focus-workspace 2
target/release/neori msg action move-column-left
target/release/neori msg action toggle-overview
```

List all actions available over IPC with `target/release/neori msg action --help` (config-only actions are hidden). Add `--json` to `neori msg` to see raw JSON, and use `RUST_LOG=neori=trace` for verbose logs.

For Lua-defined actions, put binds in a temporary config and exercise them from the nested session:

```lua
neori.bind("Mod+1", { focus_workspace = 1 })
neori.bind("Mod+2", "move-column-left")
neori.bind("Mod+3", { toggle_overview = true })
neori.bind("Mod+Shift+F", "enlarge-cursor")
```

### Upstream

Neori is a fork of [niri](https://github.com/niri-wm/niri) by YaLTeR. See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines.

### License

Neori is distributed under the terms of the **GNU General Public License v3.0 or later** (GPL-3.0-or-later). The codebase inherits this license from upstream niri.

Note: some pieces of code were adapted from other projects (e.g. **Mutter**, GPL-2.0-or-later, which is compatible with GPL-3.0 when combined as here; `src/cursor.rs` implements an **XCursorBuffer** inspired by `wayland-rs`, licensed under MIT). See individual source files for details.

**Fork notice:** Neori is a modified version of niri. This repository contains modifications as of 2025-2026. Neori is not affiliated with or endorsed by the niri project.
