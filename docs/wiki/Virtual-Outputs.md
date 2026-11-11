# Virtual Outputs

Virtual outputs are extra `wl_output`s (usually named `HEADLESS-*`) that neori can create even when
there's no physical monitor attached (or in addition to real monitors).

They're useful for:

- Sunshine / Moonlight (wlr-screencopy capture) — gaming and remote desktop
- VNC (e.g. wayvnc)
- "headless" remote sessions and general screen sharing
- Workstations with no physical display that you connect to remotely

To any client, a virtual output appears as a normal `wl_output` at the geometry/mode you give it
and generates `monitor-added` / `monitor-removed` events exactly like a physical monitor, so
`neori.on("monitor-added", ...)` handlers fire for virtual outputs too.

## Creating virtual outputs

### TTY backend (regular session with physical displays)

When running neori on a TTY with your physical monitor, you can create additional virtual outputs:

```bash
# Create a 1920x1080@144 virtual output named "sunshine"
neori msg create-virtual-output \
  --name sunshine \
  --width 1920 \
  --height 1080 \
  --refresh-rate 144

# If --name is omitted, a unique HEADLESS-N name is generated.
```

If you want to see what exists:

```bash
neori msg outputs
```

### Headless backend (no physical displays)

For servers or remote-only access (e.g. SSH) where a real TTY is not available:

```bash
# Start neori in headless mode
NEORI_BACKEND=headless neori --session &
# A 1920x1080@60 HEADLESS-1 virtual output is created by default
```

Note: if you're running commands from another shell, you may need to set `WAYLAND_DISPLAY` to the
socket of that headless session.

## Removing virtual outputs

```bash
neori msg remove-virtual-output sunshine
# Output: Removed virtual output: sunshine
```

## Configuring virtual outputs

Virtual outputs can be configured like regular outputs:

```bash
# Enable/disable
neori msg output HEADLESS-1 <on|off>

# Set scale
neori msg output HEADLESS-1 scale 1.25

# Set transform
neori msg output HEADLESS-1 transform 90
```

You can also configure them in your `config.lua` file.

If you want neori to *create* a named virtual output automatically (at compositor startup and on
config reload), add the `create_virtual` flag to the corresponding `output` block.

Note: this works only on the TTY and Headless backends (virtual outputs are not supported on the
Winit backend).

```lua
neori.output("sunshine", {
    create_virtual = true,
    mode = "1920x1080@144.000",
    scale = 1.25,
    transform = "90",
    position = { x = 1920, y = 0 },
})
```

## Using with Sunshine

If you want, you can tell [Sunshine](https://github.com/LizardByte/Sunshine) to create a virtual output that matches the client ([Moonlight](https://github.com/moonlight-stream/moonlight-qt)) resolution + refresh rate when a session starts, and remove it when the session ends.

Sunshine captures via `zwlr_screencopy_manager_v1` and injects input via `zwp_virtual_keyboard`
and `zwlr_virtual_pointer`, all of which neori advertises as Wayland globals. For dmabuf capture
(which is zero-copy from neori's GPU render target) neori also advertises `zwp_linux_dmabuf_v1`
feedback carrying its render node and format/modifier tranches — Sunshine will pick a tiled
Intel/AMD/NVIDIA format automatically when available.

The snippets below are intentionally minimal; adjust them to your Sunshine setup.

### TTY backend (create + remove per session)

```json
{
  "apps": [
    {
      "name": "Remote Desktop",
      "output": "sunshine",
      "prep-cmd": [
        {
          "do": "sh -c \"neori msg create-virtual-output --name sunshine --width ${SUNSHINE_CLIENT_WIDTH} --height ${SUNSHINE_CLIENT_HEIGHT} --refresh-rate ${SUNSHINE_CLIENT_FPS}\"",
          "undo": "neori msg remove-virtual-output sunshine"
        }
      ]
    }
  ]
}
```

#### Drain windows before destroying the output

If you want to be safe about shutting down — the Sunshine client disconnects while a game or app
is still mapped on the virtual output — move surviving windows to another workspace before
removing the output. Inspired by Noctalia Umbriel's streaming script, a prep-cmd wrapper looks
like this:

```sh
#!/bin/sh
# ~/.local/bin/neori-sunshine.sh
set -u
case "$1" in
  create)
    # Idempotent: tear down any previous session.
    neori msg remove-virtual-output sunshine 2>/dev/null || true
    neori msg create-virtual-output --name sunshine \
      --width  "${SUNSHINE_CLIENT_WIDTH:-1920}"  \
      --height "${SUNSHINE_CLIENT_HEIGHT:-1080}" \
      --refresh-rate "${SUNSHINE_CLIENT_FPS:-60}"
    ;;
  remove)
    # Give the game a few seconds to close on its own, then move any
    # leftover windows off the virtual output before we destroy it.
    for _ in $(seq 1 10); do
      sleep 0.5
      remaining=$(neori msg --json windows \
        | python3 -c 'import sys,json; d=json.load(sys.stdin); print(sum(1 for w in d if w.get("output")=="sunshine"))')
      [ "$remaining" = "0" ] && break
    done
    neori msg remove-virtual-output sunshine
    ;;
esac
```

Then use it in sunshine.conf:

```json
"prep-cmd": [
  { "do": "/home/you/.local/bin/neori-sunshine.sh create",
    "undo": "/home/you/.local/bin/neori-sunshine.sh remove" }
]
```

Because the `sunshine` output persists its workspace identity, re-creating it with the same name
later will move those remembered windows back onto it.

#### Pin the game to the virtual output

To make sure the stream-targeted app opens on the virtual output (and fullscreens), add a window
rule. Combine it with Proton's `WAYLANDDRV_PRIMARY_MONITOR` so Wine/Proton reports the virtual
output as the primary display to the game:

```lua
-- Pin anything whose app-id starts with "steam_app_" to the sunshine output, fullscreened.
neori.window_rule {
    matches = { { app_id = "^steam_app_" } },
    open_on_output = "sunshine",
    open_fullscreen = true,
}
```

In the Steam launch options for the game:

```
WAYLANDDRV_PRIMARY_MONITOR=sunshine %command%
```

### Headless backend (reuse the default output)

In headless mode, neori creates `HEADLESS-1` by default. You can just change its mode for the
session and restore it afterwards.

```json
{
  "apps": [
    {
      "name": "Remote Desktop",
      "output": "HEADLESS-1",
      "prep-cmd": [
        {
          "do": "sh -c \"neori msg output HEADLESS-1 custom-mode \\\"${SUNSHINE_CLIENT_WIDTH}x${SUNSHINE_CLIENT_HEIGHT}@${SUNSHINE_CLIENT_FPS}\\\"\"",
          "undo": "neori msg output HEADLESS-1 mode '1920x1080@60.000'"
        }
      ]
    }
  ]
}
```

## Headless workstation with no physical display

A common setup is a machine with no monitor attached that you log into remotely over Sunshine,
wayvnc, or RDP. Neori supports this directly: start it in headless mode, let it create a default
`HEADLESS-1` output, and run your remote-access server as a regular client.

### Minimal manual start

```bash
# Short runtime dir to avoid SUN_LEN limits on the unix socket.
export XDG_RUNTIME_DIR=/run/user/$(id -u)
NEORI_BACKEND=headless neori --session &

# Verify from another shell on the same machine
WAYLAND_DISPLAY=wayland-1 neori msg outputs
# -> Output "neori HEADLESS-1 Unknown" (HEADLESS-1)
#    Current mode: 1920x1080 @ 60.000 Hz (preferred)
```

Clients connecting afterwards (browsers, IDEs, games, GUI daemons, etc.) see `HEADLESS-1` as a
real `wl_output` and render to it; you capture their rendered contents from your remote-access
server.

### Persistent output declared in the config

Put a `create_virtual` output in `~/.config/neori/config.lua` so that the output exists with a
stable name/mode even after restarts — useful for Sunshine `output = "sunshine"` entries and for
`wayvnc --output sunshine`:

```lua
neori.output("sunshine", {
    create_virtual = true,
    mode = "2560x1440@165.000",
})
```

Neori creates this output at compositor startup and again whenever the config is reloaded. If
you want to stream a different resolution per Moonlight client, you can keep this entry and
still change its mode per-session with `neori msg output sunshine custom-mode ...`.

### Resizing on demand from Lua

Because `neori.create_virtual_output` / `neori.remove_virtual_output` are regular actions, you
can also script the whole thing from Lua — for example, create a Sunshine output on a hotkey
or when another condition (a laptop-lid event, a connected dock) occurs:

```lua
neori.bind("Mod+Alt+V", neori.create_virtual_output {
    name = "sunshine", width = 1920, height = 1080, refresh_rate = 144,
})
neori.bind("Mod+Alt+Shift+V", neori.remove_virtual_output("sunshine"))

-- Fire as a one-off on startup:
neori.on("startup", neori.create_virtual_output { name = "sunshine" })
```

### What was verified

The following is verified end-to-end for the headless workstation scenario on neori's current
port of upstream PR #3800:

- `NEORI_BACKEND=headless neori --session` starts cleanly with no display attached and creates
  a default `HEADLESS-1` at 1920x1080@60.
- `neori msg create-virtual-output` and `neori msg remove-virtual-output` add/remove outputs
  live; both paths fire `monitor-added` / `monitor-removed`.
- A fresh Wayland client sees the new `wl_output` with the correct `make`, `model`,
  `current`+`preferred` mode, and `xdg-output` logical geometry.
- `zwp_linux_dmabuf_v1` advertises a main device + tranche with GPU-specific modifiers (verified
  against `/dev/dri/renderD128` with Intel modifiers on the test host), so dmabuf-based capture
  by Sunshine negotiates a zero-copy format.
- `zwlr_screencopy_manager_v1`, `zwp_virtual_keyboard_manager_v1`, and
  `zwlr_virtual_pointer_manager_v1` are present — the Wayland globals Sunshine uses for capture
  and input injection.

### Caveats on a true headless host

- A GPU is still useful: the headless backend creates an EGL/GLES renderer for compositing; in
  practice this works with any DRM render node, including a software renderer (llvmpipe) if no
  GPU is present, but zero-copy dmabuf screencopy requires a real render node.
- On some graphics drivers the renderer cannot bind `wl_display` (missing
  `EGL_WL_bind_wayland_display`). That path is only used for `wl_shm` upload hints; dmabuf
  clients and dmabuf-based screencopy (Sunshine's fast path) are unaffected.
- The compositor's Wayland socket lives under `$XDG_RUNTIME_DIR`. Keep that path short (unix
  socket `SUN_LEN` limit) — `/run/user/$(id -u)` as used by most distros is fine.

## Input and seats (headless mode)

In headless mode, neori can still use libinput to read local input devices (if it has permission to
open `/dev/input/event*`). This is independent of virtual outputs.

libinput enumerates devices by udev seat. By default neori uses `seat0`, but you can override it:

```bash
XDG_SEAT=seat0 NEORI_BACKEND=headless neori --session
```

The Wayland `wl_seat` name exposed by neori matches this seat string (`XDG_SEAT` / `seat0`).
If neori can't access any input devices (permissions, container, etc.), it will still start; you'll
just have no _local_ input.

This section is mostly about _local_ kernel input devices. If you're using a remote client that
injects input over Wayland (like wayvnc), neori doesn't need access to `/dev/input` for that.

## Using with wayvnc

[wayvnc](https://github.com/any1/wayvnc) is a VNC server for wlroots-based Wayland compositors. It forwards keyboard and pointer input from VNC clients to the compositor using Wayland "virtual input" protocols (virtual keyboard + virtual pointer). This means you can have working remote input even if the headless neori process can't open `/dev/input/event*`.

### Physical displays + extra virtual output

```bash
# 1. Start neori normally on your TTY
neori --session &

# 2. Create a virtual output for VNC
neori msg create-virtual-output --width 1920 --height 1080

# 3. Start wayvnc on the virtual output
wayvnc --output HEADLESS-1

# 4. Connect from a VNC client to your machine's IP
```

### Pure headless (remote only)

```bash
# 1. Start neori in headless mode (e.g., over SSH)
NEORI_BACKEND=headless neori &

# 2. Start wayvnc
WAYLAND_DISPLAY=wayland-1 wayvnc --output HEADLESS-1

# 3. Connect from a VNC client
```

### Headless with systemd

For a persistent headless neori session:

```ini
# ~/.config/systemd/user/neori-headless.service
[Unit]
Description=Neori Headless Session

[Service]
Type=simple
Environment=NEORI_BACKEND=headless
ExecStart=/usr/bin/neori
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now neori-headless
```
