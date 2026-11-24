## Running a Local Build

The main way of testing neori during development is running it as a nested window. The second step is usually switching to a different TTY and running neori there.

Once a feature or fix is reasonably complete, you generally want to run a local build as your main compositor for proper testing. The easiest way to do that is to install neori normally (from a distro package for example), then overwrite the binary with `sudo cp ./target/release/neori /usr/bin/neori`. Do make sure that you know how to revert to a working version in case everything breaks though.

If you use an RPM-based distro, you can generate an RPM package for a local build with `cargo generate-rpm`.

## Logging Levels

Neori uses [`tracing`](https://lib.rs/crates/tracing) for logging. This is how logging levels are used:

- `error!`: programming errors and bugs that are recoverable. Things you'd normally use `unwrap()` for. However, when a Wayland compositor crashes, it brings down the entire session, so it's better to recover and log an `error!` whenever reasonable. If you see an `ERROR` in the neori log, that always indicates a *bug*.
- `warn!`: something bad but still *possible* happened. Informing the user that they did something wrong, or that their hardware did something weird, falls into this category. For example, config parsing errors should be indicated with a `warn!`.
- `info!`: the most important messages related to normal operation. Running neori with `RUST_LOG=neori=info` should not make the user want to disable logging altogether.
- `debug!`: less important messages related to normal operation. Running neori with `debug!` messages hidden should not negatively impact the UX.
- `trace!`: everything that can be useful for debugging but is otherwise too spammy or performance intensive. `trace!` messages are *compiled out* of release builds.

## Tests

We have some unit tests, most prominently for the layout code and for config parsing.

When adding new operations to the layout, add them to the `Op` enum at the bottom of `src/layout/mod.rs` (this will automatically include it in the randomized tests), and if applicable to the `every_op` arrays below.

When adding new config options, include them in the config parsing test.

Config structs derive `LuaTable` (from the `neori-config-derive` crate), so a field added to one
is a Lua config key right away, in snake_case and kebab-case, parsed by its type's
`FromLuaConfig` implementation in `neori-config/src/lua_table.rs`. A new value type needs one
there. The editor definitions of the config tables, `resources/lua/neori-config.lua`, are
generated from the same structs; the `lua-definitions` test fails until you regenerate them:

```sh
NEORI_UPDATE_LUA_DEFINITIONS=1 cargo test -p neori-config --test lua-definitions
```

The wiki's [Lua API Reference](./Lua-API-Reference.md) is generated from the API's definitions,
`resources/lua/neori.lua`, so a change there needs it regenerated too; the `lua-reference` test
fails until you do:

```sh
NEORI_UPDATE_LUA_REFERENCE=1 cargo test -p neori-config --test lua-reference
```

Keep the definitions' layout: `----` banners make the reference's sections, a `-- Title`
comment makes a subsection (any `--` lines after it are its text), and each `---` doc block goes
right before the declaration it describes.

### Running Tests

Make sure to run `cargo test --all` to run tests from sub-crates too.

Some tests are a bit too slow to run normally, like the randomized tests of the layout code, so they are normally skipped. Set the `RUN_SLOW_TESTS` variable to run them:

```
env RUN_SLOW_TESTS=1 cargo test --all
```

It also usually helps to run the randomized tests for a longer period, so that they can explore more inputs. You can control this with environment variables. This is how I usually run tests before pushing:

```
env RUN_SLOW_TESTS=1 PROPTEST_CASES=200000 PROPTEST_MAX_GLOBAL_REJECTS=200000 RUST_BACKTRACE=1 cargo test --release --all
```

### Lua Tests

Tests of the Lua API can be written in Lua: each `src/tests/lua/*.lua` file is loaded as the
config of a test compositor, and its `run()` function is the test. The file's top level sets up
handlers, rules and binds like a config, and `run()` drives the compositor with the `test` table
and checks the results with the live API:

```lua
neori.rules.add { where = { app_id = "^pip$" }, set = { floating = true } }

function run()
    test.output()
    local w = test.window { app_id = "pip" }
    assert(w:is_floating())
end
```

`test.output(width, height)` adds an output, `test.window { app_id, title }` opens a window and
returns it, `test.title(w, title)` and `test.close(w)` act like its app, `test.press(key)` presses
a bind, `test.type()` types an unbound key, `test.refresh(n)` runs the event loop, and
`test.lua_log()` returns what callbacks printed and the errors they raised. `test.eq(a, b)`
asserts equality. They run with the other tests; `NEORI_LUA_TEST=rules cargo test
lua_script_tests` runs the files whose name contains `rules`. The harness is
`src/tests/lua_scripts.rs`.

### Visual Tests

The `neori-visual-tests` sub-crate is a GTK application that runs hard-coded test cases so that you can visually check that they look right. It uses mock windows with the real layout and rendering code. It is especially helpful when working on animations.

## Profiling

We have integration with the [Tracy](https://github.com/wolfpld/tracy) profiler which you can enable by building neori with a feature flag:

```
cargo build --release --features=profile-with-tracy-ondemand
```

Before starting Tracy-integrated neori, run the following commands to give Tracy access to CPU scheduling and context switches (can be very useful for diagnosing slowdowns):

```
sudo mount -o remount,gid=$(id -g),mode=750 /sys/kernel/tracing; echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

Then you can open Tracy (you will need the latest stable release) and attach to a running neori instance to collect profiling data. Profiling data is collected "on demand"—that is, only when Tracy is connected. You can run a neori build like this as your main compositor if you'd like.

> [!NOTE]
> If you need to profile neori startup or the neori CLI, you can opt for "always on" profiling instead, using this feature flag:
>
> ```
> cargo build --release --features=profile-with-tracy
> ```
>
> When compiled this way, neori will **always** collect profiling data, so you can't run a build like this as your main compositor.

To make a neori function show up in Tracy, instrument it like this:

```rust
pub fn some_function() {
    let _span = tracy_client::span!("some_function");

    // Code of the function.
}
```

You can also enable Rust memory allocation profiling with `--features=profile-with-tracy-allocations`.
