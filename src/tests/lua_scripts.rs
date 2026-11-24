//! Integration tests written in Lua: each `src/tests/lua/*.lua` file is loaded as the config of a
//! test compositor, and its `run()` function is the test.
//!
//! The file's top level runs like a config, so it sets up handlers, rules and binds. `run()` then
//! drives the compositor with the `test` table and checks the results with the live API and
//! `assert`:
//!
//! ```lua
//! neori.rules.add { where = { app_id = "^pip$" }, set = { floating = true } }
//!
//! function run()
//!     test.output(1920, 1080)
//!     local w = test.window { app_id = "pip" }
//!     assert(w:is_floating())
//! end
//! ```
//!
//! `run()` runs as a coroutine. Each `test.*` call that needs the compositor yields a command,
//! which this harness carries out before resuming it with the result. Between those calls the
//! code runs like a callback: reads are live and changes apply right away. See the `PRELUDE` for
//! the commands.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use neori_config::binds::Key;
use neori_config::mlua::{self, Value};
use neori_config::Config;
use smithay::backend::input::InputTime;
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;

use super::client::ClientId;
use super::*;

/// The `test` table, defined after the file loaded.
const PRELUDE: &str = r#"
local yield = coroutine.yield
test = {}

-- Adds an output of this size and returns its connector name.
function test.output(width, height)
    return yield({ op = "output", width = width or 1920, height = height or 1080 })
end

-- Opens a window with { app_id = …, title = … }, runs the event loop until it's shown, and
-- returns its Window.
function test.window(props)
    props = props or {}
    local id = yield({ op = "window", app_id = props.app_id, title = props.title })
    for _, w in ipairs(neori.windows()) do
        if w:id() == id then return w end
    end
    error("the new window " .. tostring(id) .. " isn't in neori.windows()", 2)
end

-- Changes a window's title, like its app would.
function test.title(w, title)
    yield({ op = "title", window = w:id(), title = title })
end

-- Closes a window, like its app would.
function test.close(w)
    yield({ op = "close", window = w:id() })
end

-- Runs the event loop: refreshes, property signals, idle callbacks. 1 time by default.
function test.refresh(times)
    yield({ op = "refresh", times = times or 1 })
end

-- Presses and releases the bind for this key, like the user would.
function test.press(key)
    yield({ op = "press", key = key })
end

-- Presses and releases an unbound key: user input, which ends chains of Lua-caused signals.
function test.type()
    yield({ op = "type" })
end

-- What has been written to the Lua log: prints and errors of callbacks.
function test.lua_log()
    return yield({ op = "lua_log" })
end

-- Asserts that a == b, showing both when they differ.
function test.eq(a, b, message)
    if a ~= b then
        error((message and message .. ": " or "") .. tostring(a) .. " ~= " .. tostring(b), 2)
    end
end
"#;

/// The Lua log of a test, readable from Rust.
#[derive(Clone, Default)]
struct TestLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TestLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Harness {
    f: Fixture,
    client: Option<ClientId>,
    /// The surface of each window the test opened, by window id.
    windows: HashMap<u64, ClientWlSurface>,
    outputs: u8,
    log: TestLog,
}

/// A value handed back to `run()`, written as Lua.
enum Reply {
    Nil,
    Int(u64),
    Str(String),
}

impl Reply {
    fn lua(&self) -> String {
        match self {
            Reply::Nil => "nil".to_owned(),
            Reply::Int(n) => n.to_string(),
            Reply::Str(s) => format!("{s:?}"),
        }
    }
}

impl Harness {
    fn new(path: &Path, code: &str) -> Result<Self, String> {
        let (config, host) = Config::parse_with_host(path, code)
            .config
            .map_err(|e| format!("{e:?}"))?;
        let mut f = Fixture::with_config(config);
        f.state.server.state.install_script_host(host);

        let log = TestLog::default();
        let sink: neori_config::PrintSink = Arc::new(Mutex::new(Box::new(log.clone())));
        let state = &mut f.state.server.state;
        state.scripting.set_print_log(Arc::clone(&sink));
        state.print_log_sink = Some(sink);

        Ok(Self {
            f,
            client: None,
            windows: HashMap::new(),
            outputs: 0,
            log,
        })
    }

    fn eval(&mut self, code: &str) -> Result<Vec<String>, String> {
        self.f
            .state
            .server
            .state
            .eval_lua(code)
            .map(|output| output.values)
    }

    fn global(&self, name: &str) -> Value {
        self.f
            .state
            .server
            .state
            .scripting
            .read_global(name)
            .unwrap()
    }

    fn refresh(&mut self) {
        self.f.state.server.dispatch();
        self.f.state.server.dispatch();
    }

    fn client(&mut self) -> ClientId {
        match self.client {
            Some(id) => id,
            None => {
                let id = self.f.add_client();
                self.client = Some(id);
                id
            }
        }
    }

    fn window_ids(&mut self) -> Vec<u64> {
        self.f
            .neori()
            .layout
            .windows()
            .map(|(_, w)| w.id().get())
            .collect()
    }

    /// Carries out a command from `run()`.
    fn command(&mut self, cmd: &mlua::Table) -> Result<Reply, String> {
        let get_str = |key: &str| cmd.get::<Option<String>>(key).map_err(|e| e.to_string());
        let get_int = |key: &str| cmd.get::<Option<u64>>(key).map_err(|e| e.to_string());
        let op = get_str("op")?.unwrap_or_default();
        match op.as_str() {
            "output" => {
                self.outputs += 1;
                let n = self.outputs;
                let size = (
                    get_int("width")?.unwrap_or(1920) as u16,
                    get_int("height")?.unwrap_or(1080) as u16,
                );
                self.f.add_output(n, size);
                if n == 1 {
                    self.f.neori_focus_output(1);
                }
                self.refresh();
                Ok(Reply::Str(format!("headless-{n}")))
            }
            "window" => {
                let before = self.window_ids();
                let id = self.client();
                let window = self.f.client(id).create_window();
                if let Some(app_id) = get_str("app_id")? {
                    window.xdg_toplevel.set_app_id(app_id);
                }
                if let Some(title) = get_str("title")? {
                    window.set_title(&title);
                }
                let surface = window.surface.clone();
                window.commit();
                self.f.roundtrip(id);
                let window = self.f.client(id).window(&surface);
                window.attach_new_buffer();
                window.set_size(200, 200);
                window.ack_last_and_commit();
                self.f.double_roundtrip(id);
                self.refresh();

                let new = self
                    .window_ids()
                    .into_iter()
                    .find(|id| !before.contains(id))
                    .ok_or("the window didn't open")?;
                self.windows.insert(new, surface);
                Ok(Reply::Int(new))
            }
            "title" | "close" => {
                let window = get_int("window")?.ok_or("no window")?;
                let surface = self
                    .windows
                    .get(&window)
                    .cloned()
                    .ok_or_else(|| format!("window {window} wasn't opened by test.window"))?;
                let id = self.client();
                let client_window = self.f.client(id).window(&surface);
                if op == "title" {
                    client_window.set_title(&get_str("title")?.unwrap_or_default());
                } else {
                    client_window.attach_null();
                }
                client_window.commit();
                self.f.double_roundtrip(id);
                self.refresh();
                Ok(Reply::Nil)
            }
            "refresh" => {
                for _ in 0..get_int("times")?.unwrap_or(1) {
                    self.refresh();
                }
                Ok(Reply::Nil)
            }
            "press" => {
                let key: Key = get_str("key")?
                    .unwrap_or_default()
                    .parse()
                    .map_err(|e| format!("{e}"))?;
                // Binds match the physical key by default, so the config may hold the keycode.
                let keycode = key.try_into_keycode();
                let bind = {
                    let config = self.f.state.server.state.neori.config.borrow();
                    config
                        .binds
                        .0
                        .iter()
                        .find(|b| b.key == key || Some(b.key) == keycode)
                        .cloned()
                };
                let bind = bind.ok_or("no bind for that key")?;
                let state = &mut self.f.state.server.state;
                state.handle_bind(bind.clone(), true, InputTime::now());
                state.handle_bind(bind, false, InputTime::now());
                self.refresh();
                Ok(Reply::Nil)
            }
            "type" => {
                type_key(&mut self.f);
                Ok(Reply::Nil)
            }
            "lua_log" => Ok(Reply::Str(
                String::from_utf8_lossy(&self.log.0.lock().unwrap()).into_owned(),
            )),
            _ => Err(format!("unknown test command {op:?}")),
        }
    }

    /// Runs `run()` to the end.
    fn run(&mut self) -> Result<(), String> {
        self.eval(PRELUDE)?;
        if !matches!(self.global("run"), Value::Function(_)) {
            return Err("the file defines no run() function".to_owned());
        }
        self.eval("__test_co = coroutine.create(run)")?;

        let mut reply = Reply::Nil;
        loop {
            let status = self.eval(&format!(
                r#"
                local ok, cmd = coroutine.resume(__test_co, {})
                if not ok then error(cmd, 0) end
                __test_cmd = cmd
                return coroutine.status(__test_co)
                "#,
                reply.lua()
            ))?;
            if status.first().map(String::as_str) == Some("dead") {
                return Ok(());
            }
            let Value::Table(cmd) = self.global("__test_cmd") else {
                return Err("run() yielded something other than a test command".to_owned());
            };
            reply = self.command(&cmd)?;
        }
    }
}

/// Presses and releases F12, which no bind uses.
fn type_key(f: &mut Fixture) {
    use smithay::backend::input::{InputEvent, KeyState, Keycode};

    use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

    for state in [KeyState::Pressed, KeyState::Released] {
        f.neori_state()
            .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                event: TestKeyboardKeyEvent {
                    time: InputTime::from_micros(0),
                    code: Keycode::new(96),
                    state,
                    count: 1,
                },
            });
    }
}

#[test]
fn lua_script_tests() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/lua");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "lua"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no tests in {}", dir.display());

    let mut failures = String::new();
    for path in &files {
        let code = std::fs::read_to_string(path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        // A file can be run alone: NEORI_LUA_TEST=rules cargo test lua_script_tests
        if let Ok(only) = std::env::var("NEORI_LUA_TEST") {
            if !name.contains(&only) {
                continue;
            }
        }
        if let Err(err) = Harness::new(path, &code).and_then(|mut h| h.run()) {
            writeln!(failures, "\n--- {name}\n{err}").unwrap();
        }
    }
    assert!(failures.is_empty(), "Lua tests failed:{failures}");
}
