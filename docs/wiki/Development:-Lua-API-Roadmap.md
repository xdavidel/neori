This page tracks the plan for growing neori's Lua API, borrowing the good parts of
[cwc](https://github.com/Cudiph/cwcwm)'s awesomewm-style Lua API.

The goal is to adopt cwc's *semantics* (live objects, signals, timers, modes, Lua over IPC) while
keeping neori's names (`neori.windows()`, `Window`, `Output`, `Workspace`) and its scrolling
layout model.
Existing APIs keep working as aliases until the new ones fully replace them.

### What we are not adopting

- Tags, master/stack/bsp layouts and containers: they don't map onto niri's scrolling columns.
- C plugins: Rust has no stable ABI to load them against.
- The wibox/cairo widget toolkit and title bars: out of scope, shells like noctalia cover this.
- A global `key-pressed` signal: it would hand every keystroke to Lua. At most an opt-in for debugging.

### Phase 0: foundations (done)

- **Live handles.**
  A `Window`, `Output` or `Workspace` holds an id instead of a snapshot.
  Reading a property looks up the current compositor state, and setters and methods apply directly
  rather than being queued as actions.
  Callbacks run with the compositor state lent to them through a script context.
- **Signals.**
  One registry with `connect`, `disconnect` and `emit`, both global and per object, including
  user-defined signals.
  `neori.on(event, fn)` stays as an alias for connecting a global signal.
- **No cost when unused.**
  Firing a signal that has no Lua handler must stay free, like the existing event helpers.
- **LuaLS type stubs** for editor completion, in `resources/lua/neori.lua`. They're written by hand,
  and a test checks them against the real API.

### Phase 1: high value features (done)

- **Timers (done):** `neori.after(ms, fn)` and `neori.every(ms, fn)`, on calloop timers. (Not
  `neori.timer { repeat = … }`: `repeat` is a Lua keyword.)
- **Property signals (done):** `window-title-changed`, `window-fullscreen-changed`,
  `window-floating-changed`, `window-urgent-changed`, `window-workspace-changed`,
  `workspace-name-changed`, `output-mode-changed` (`title-changed` and so on on the objects),
  with the old value. They come from their own per-frame diff of what Lua reads rather than the
  IPC diff, which compares the fullscreen state the client acknowledged, and only runs with an
  IPC server.
- **Keybinding modes (done):** `neori.mode("resize", function() … end)` (binds made inside go into
  the mode), `enter-mode` / `exit-mode` actions, `neori.enter_mode`, `neori.exit_mode`,
  `neori.current_mode`, a `mode-changed` signal, and the hotkey overlay shows the active mode.
- **Lua over IPC (done):** `neori msg lua '<expr>'` and `neori msg lua -f script.lua`.
  The socket is only accessible to the same user, like the existing `spawn` action.
- **Lifecycle (done):** `neori.is_startup()`, plus `config-reloaded` and `shutdown` signals.

### Phase 2: wider object coverage

- **Window (done, except `opacity` and `parent`):** `pid`, `geometry`, `is_urgent`, `is_maximized`,
  `maximize()`, `kill()`, `center()`, and `neori.window_at(x, y)`.
- **Output (done):** `geometry`, `workarea`, `physical_size`, `windows()`, and
  `neori.output_at(x, y)`; `is_off` covers enabled. Monitors are powered on and off together, by
  the `power-off-monitors` / `power-on-monitors` actions, so there's no per-output DPMS.
- **Input devices (done, except `map_to_output`):** `device-added` and `device-removed` signals, a
  per-device `removed` signal, and settable `accel_profile`, `scroll_method`, `click_method`, `dwt`
  and `middle_emulation`. `map_to_output` maps tablets and touch screens in the compositor rather
  than in libinput, so it needs its own work.
- **Pointer:** button, axis and gesture (swipe, pinch, hold) signals, opt-in and rate aware, and
  starting an interactive move or resize from Lua.
- **Layer surfaces:** a `LayerSurface` object (`namespace`, `pid`, `output`, `kill()`) with add and
  remove signals.
- **Environment (done):** `neori.setenv`, `neori.unsetenv` and `neori.getenv` at runtime.

### Phase 3: libraries and developer experience

- `neori.util`: table, string and geometry helpers ported from awesome's `gears`
  (GPL-2.0-or-later, notices kept).
- `neori.rules`: match window properties and run a callback, built on window-opening and property
  signals.
- Lua-script-driven integration tests, and a strict-mode wiki test.
- Generated API reference docs.

### Known gaps to fold in

These Lua config options are documented but not parsed yet, and get fixed along the way:
recent-windows actions, most `debug` keys, the window-rule `scroll_factor`, most layer-rule
options, `on` in focus ring / insert hint / tab indicator, `place_within_column`, the
`switch-layout` action, and `focus = false` on move actions.

### Risks

- Phase 0 is a big refactor of `src/script.rs` and `neori-config/src/parse_lua.rs`.
  The snapshot API keeps working until the live one is complete.
- Re-entrancy: a setter that changes the layout inside a signal handler can fire more signals.
  Signal emission needs a re-entrancy guard and a depth limit.
- Hot paths: pointer and property signals must not slow down the compositor when nobody listens.
  This is covered by perf tests like the existing ones in `src/tests/perf.rs`.
