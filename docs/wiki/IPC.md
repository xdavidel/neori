You can communicate with the running neori instance over an IPC socket.
Check `neori msg --help` for available commands.

The `--json` flag prints the response in JSON, rather than formatted.
For example, `neori msg --json outputs`.

> [!TIP]
> If you're getting parsing errors from `neori msg` after upgrading neori, make sure that you've restarted neori itself.
> You might be trying to run a newer `neori msg` against an older `neori` compositor.

### Running Lua

`neori msg lua` runs Lua code in the compositor, in your config's Lua state, and prints what it
returns. See [Lua Scripting](./Lua-Scripting.md#running-lua-from-a-shell).

```sh
neori msg lua 'neori.focused_window():title()'
```

### Event Stream

<sup>Since: 0.1.9</sup>

While most neori IPC requests return a single response, the event stream request will make neori continuously stream events into the IPC connection until it is closed.
This is useful for implementing various bars and indicators that update as soon as something happens, without continuous polling.

The event stream IPC is designed to give you the complete current state up-front, then follow up with updates to that state.
This way, your state can never "desync" from neori, and you don't need to make any other IPC information requests.

Where reasonable, event stream state updates are atomic, though this is not always the case.
For example, a window may end up with a workspace id for a workspace that had already been removed.
This can happen if the corresponding workspaces-changed event arrives before the corresponding window-changed event.

To get a taste of the events, run `neori msg event-stream`.
Though, this is more of a debug function than anything.
You can get raw events from `neori msg --json event-stream`, or by connecting to the neori socket and requesting an event stream manually.

You can find the full list of events along with documentation [here](https://xdavidel.github.io/neori/neori_ipc/enum.Event.html).

### Programmatic Access

`neori msg --json` is a thin wrapper over writing and reading to a socket.
When implementing more complex scripts and modules, you're encouraged to access the socket directly.

Connect to the UNIX domain socket located at `$NEORI_SOCKET` in the filesystem.
Write your request encoded in JSON on a single line, followed by a newline character, or by flushing and shutting down the write end of the connection.
Read the reply as JSON, also on a single line.

You can use `socat` to test communicating with neori directly:

```sh
$ socat STDIO "$NEORI_SOCKET"
"FocusedWindow"
{"Ok":{"FocusedWindow":{"id":12,"title":"t socat STDIO /run/u ~","app_id":"Alacritty","workspace_id":6,"is_focused":true}}}
```

The reply is an `Ok` or an `Err` wrapping the same JSON object as you get from `neori msg --json`.

<sup>Since: 0.2.0</sup>
For more complex requests, you can pass `--print-request` to `neori msg` to see how the request should be formatted:

```sh
$ neori msg --print-request action focus-workspace 2
{"Action":{"FocusWorkspace":{"reference":{"Index":2}}}}
```

This is the format that you should use for communicating with the neori socket directly.
You can also use `neori msg raw-request` to send a raw JSON request without `socat`:

```sh
$ echo '{"Action":{"FocusWorkspace":{"reference":{"Id":8}}}}' | neori msg raw-request
"Handled"
```

You can find all available requests and response types in the [neori-ipc sub-crate documentation](https://xdavidel.github.io/neori/neori_ipc/).

### Backwards Compatibility

The JSON output *should* remain stable, as in:

- existing fields and enum variants should not be renamed
- non-optional existing fields should not be removed

However, new fields and enum variants will be added, so you should handle unknown fields or variants gracefully where reasonable.

The formatted/human-readable output (i.e. without `--json` flag) is **not** considered stable.
Please prefer the JSON output for scripts, since I reserve the right to make any changes to the human-readable output.

The `neori-ipc` sub-crate (like other neori sub-crates) is *not* API-stable in terms of the Rust semver; rather, it follows the version of neori itself.
In particular, new struct fields and enum variants will be added.
