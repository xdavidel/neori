<h1 align="center">neori</h1>

<p align="center">
    <a href="https://www.gnu.org/licenses/gpl-3.0.html"><img alt="License: GPL-3.0-or-later" src="https://img.shields.io/badge/License-GPLv3%2B-blue.svg"></a>
</p>

Neori is a **scrollable-tiling Wayland compositor** — a fork of [niri](https://github.com/niri-wm/niri) with extended configuration support.

Windows are arranged in columns on an infinite horizontal strip. Opening a new window never causes existing windows to resize. Scroll left and right through your workspace.

### Neori-specific features

- **Lua-based configuration** — configure with `config.lua` instead of KDL
- **Window-rule extensions** — `geometry_corner_radius`, `clip_to_geometry`, `draw_border_with_background`, `opacity`, `tiled_state`, `baba_is_float`, `variable_refresh_rate` and more
- **NIRI_SOCKET compatibility** — IPC remains niri-compatible (`niri msg ...` works, DMS can detect via `NIRI_SOCKET`)
- **Sparse on-demand workspace numbering** — focusing a workspace doesn't auto-create intermediate gaps
- **Config validation and API** — full `neori.*` Lua API with `startup`/`monitor-added`/`monitor-removed` hooks

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

### Upstream

Neori is a fork of [niri](https://github.com/niri-wm/niri) by YaLTeR. See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines.

### License

Neori is distributed under the terms of the **GNU General Public License v3.0 or later** (GPL-3.0-or-later). The codebase inherits this license from upstream niri.

Note: some pieces of code were adapted from other projects (e.g. **Mutter**, GPL-2.0-or-later, which is compatible with GPL-3.0 when combined as here; `src/cursor.rs` implements an **XCursorBuffer** inspired by `wayland-rs`, licensed under MIT). See individual source files for details.

**Fork notice:** Neori is a modified version of niri. This repository contains modifications as of 2025-2026. Neori is not affiliated with or endorsed by the niri project.
