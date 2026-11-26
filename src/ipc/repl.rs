//! The Lua REPL of `neori msg lua` (or `neori repl`) without code.
//!
//! Each entry runs in the compositor's Lua state, like `neori msg lua 'code'`: it shares the
//! config's globals, reads are live and changes apply right away. On top of that, the REPL keeps
//! what was typed for completion, continues unfinished code on the next line, turns a leading
//! `local` into a global so later entries see it, and shows what callbacks print (the Lua log)
//! while it's open.

use std::fs::File;
use std::io::{BufRead, BufReader, IsTerminal, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use directories::ProjectDirs;
use neori_config::mlua;
use neori_ipc::socket::Socket;
use neori_ipc::{Action, Request, Response};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::{DefaultHistory, History as _};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{CompletionType, Config, Context, Editor, ExternalPrinter as _, Helper};

use super::client::lua_table_keys;
use super::lua_complete;

const HELP: &str = "\
Lua code runs in the compositor, like `neori msg lua`: it shares the config's globals, and
changes apply right away. Tab completes, and unfinished code continues on the next line.
A leading `local` becomes a global, so the next entries see it.

  .help           this help
  .doc NAME       the documentation of a function: .doc neori.bind, .doc Window:title
  .signals        the signals and how many handlers each has
  .load FILE      run a Lua file
  .log on|off     show what callbacks print (on by default)
  .reload         reload the config file, which resets the Lua state
  .quit           leave (or Ctrl+D)";

/// ANSI styles, or nothing when the output isn't a terminal.
#[derive(Clone, Copy)]
struct Style {
    on: bool,
}

impl Style {
    fn paint(self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
    fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }
    fn red(self, text: &str) -> String {
        self.paint("31", text)
    }
    fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }
}

struct ReplHelper {
    /// Everything entered so far, so completion knows the variables it defined.
    session: std::cell::RefCell<String>,
    style: Style,
}

impl ReplHelper {
    /// The session's code followed by `line`, as the text to complete.
    fn context(&self, line: &str) -> String {
        let session = self.session.borrow();
        if session.is_empty() {
            line.to_owned()
        } else {
            format!("{session}\n{line}")
        }
    }
}

/// The length in bytes of the name being typed at the end of `text`.
fn partial_name_len(text: &str) -> usize {
    text.chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .map(char::len_utf8)
        .sum()
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if line.trim_start().starts_with('.') {
            return Ok((pos, Vec::new()));
        }
        let before = &line[..pos];
        let partial = partial_name_len(before);
        let context = self.context(before);
        // Each candidate is the whole text with the name completed.
        let keep = context.len() - partial;
        let candidates = lua_complete::complete(&context, &lua_table_keys)
            .into_iter()
            .filter_map(|candidate| candidate.get(keep..).map(str::to_owned))
            .map(|name| Pair {
                display: name.clone(),
                replacement: name,
            })
            .collect();
        Ok((pos - partial, candidates))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() {
            return None;
        }
        lua_complete::signature_hint(&self.context(line))
    }
}

impl Highlighter for ReplHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        self.style.dim(hint).into()
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> std::borrow::Cow<'b, str> {
        self.style.bold(prompt).into()
    }
}

impl Validator for ReplHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        if input.trim_start().starts_with('.') || !is_incomplete(input) {
            Ok(ValidationResult::Valid(None))
        } else {
            Ok(ValidationResult::Incomplete)
        }
    }
}

impl Helper for ReplHelper {}

/// Whether `code` is unfinished Lua, like an open `function … ` or table, which continues on the
/// next line. Code with other errors is complete: running it shows the error.
pub fn is_incomplete(code: &str) -> bool {
    let lua = mlua::Lua::new();
    // An expression is complete, like `1 + 1`, which isn't a statement.
    if lua.load(format!("return {code}")).into_function().is_ok() {
        return false;
    }
    matches!(
        lua.load(code).into_function(),
        Err(mlua::Error::SyntaxError {
            incomplete_input: true,
            ..
        })
    )
}

/// Turns a leading `local` into a global, so that the next entries see the variable:
/// `local w = …` becomes `w = …`, `local function f` becomes `function f`.
pub fn globalize_leading_local(code: &str) -> String {
    let indent = code.len() - code.trim_start().len();
    let (lead, rest) = code.split_at(indent);
    if let Some(function) = rest.strip_prefix("local function ") {
        return format!("{lead}function {function}");
    }
    let Some(names) = rest.strip_prefix("local ") else {
        return code.to_owned();
    };
    // `local x <const> = 1` keeps its local: an attribute doesn't fit a global.
    if names.split('=').next().unwrap_or("").contains('<') {
        return code.to_owned();
    }
    if names.contains('=') {
        format!("{lead}{names}")
    } else {
        // `local x` with no value.
        let names = names.trim_end().trim_end_matches(';');
        format!("{lead}{names} = nil")
    }
}

/// Where the compositor writes the Lua log, like `main.rs` opens it.
fn lua_log_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "neori")?;
    let dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
    Some(dir.join("lua.log"))
}

fn history_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "neori")?;
    let dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
    Some(dir.join("repl_history"))
}

/// A line of the Lua log, for the REPL: without its timestamp, errors in red.
fn format_log_line(line: &str, style: Style) -> String {
    let text = match line.strip_prefix('[') {
        Some(rest) => rest.split_once("] ").map_or(line, |(_, text)| text),
        None => line,
    };
    if text.starts_with("error: ") {
        style.red(text)
    } else {
        format!("{} {text}", style.dim("│"))
    }
}

/// Follows the Lua log from its current end, handing each new line to `print`.
fn follow_log(path: &Path, shown: Arc<AtomicBool>, mut print: impl FnMut(String)) {
    let open = || {
        let mut file = File::open(path).ok()?;
        let end = file.seek(SeekFrom::End(0)).ok()?;
        Some((BufReader::new(file), end))
    };
    let mut current = open();
    loop {
        let Some((reader, offset)) = current.as_mut() else {
            std::thread::sleep(Duration::from_millis(500));
            current = open();
            continue;
        };
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
                // Truncated: start again from its beginning.
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if len < *offset {
                    if let Ok(file) = File::open(path) {
                        current = Some((BufReader::new(file), 0));
                    }
                }
            }
            Ok(n) => {
                *offset += n as u64;
                if shown.load(Ordering::Relaxed) {
                    print(line.trim_end_matches('\n').to_owned());
                }
            }
        }
    }
}

/// The connection to the compositor, reconnecting when it went away.
struct Connection {
    socket: Option<Socket>,
}

impl Connection {
    fn send(&mut self, request: Request) -> anyhow::Result<Result<Response, String>> {
        for attempt in 0..2 {
            if self.socket.is_none() {
                self.socket =
                    Some(Socket::connect().context("error connecting to the neori socket")?);
            }
            match self.socket.as_mut().unwrap().send(request.clone()) {
                Ok(reply) => return Ok(reply),
                // The compositor restarted: connect again once.
                Err(_) if attempt == 0 => self.socket = None,
                Err(err) => return Err(err).context("error talking to neori"),
            }
        }
        unreachable!()
    }

    /// Runs Lua code, returning what it printed and its values, or its error.
    fn lua(&mut self, code: String) -> anyhow::Result<Result<(String, Vec<String>), String>> {
        Ok(match self.send(Request::Lua { code })? {
            Ok(Response::LuaEvaluated(result)) => Ok((result.printed, result.values)),
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(err) => Err(err),
        })
    }
}

fn show_result(result: Result<(String, Vec<String>), String>, style: Style) {
    match result {
        Ok((printed, values)) => {
            print!("{printed}");
            for value in values {
                println!("{value}");
            }
        }
        Err(err) => println!("{}", style.red(&format!("error: {err}"))),
    }
}

/// Runs the REPL until Ctrl+D or `.quit`.
pub fn run() -> anyhow::Result<()> {
    let style = Style {
        on: std::io::stdout().is_terminal(),
    };
    let mut connection = Connection { socket: None };
    // Fail early when there's no compositor to talk to; only the connection matters here.
    let _ = connection.send(Request::Version)?;

    let config = Config::builder()
        .completion_type(CompletionType::List)
        .auto_add_history(false)
        .build();
    let mut editor: Editor<ReplHelper, DefaultHistory> = Editor::with_config(config)?;
    editor.set_helper(Some(ReplHelper {
        session: Default::default(),
        style,
    }));
    let history = history_path();
    if let Some(path) = &history {
        let _ = editor.load_history(path);
    }

    let log_shown = Arc::new(AtomicBool::new(true));
    if let (Some(path), Ok(mut printer)) = (lua_log_path(), editor.create_external_printer()) {
        let shown = Arc::clone(&log_shown);
        std::thread::spawn(move || {
            follow_log(&path, shown, |line| {
                let _ = printer.print(format_log_line(&line, style));
            });
        });
    }

    println!(
        "{} {}",
        style.bold("neori Lua REPL."),
        style.dim("Tab completes, .help lists the commands, Ctrl+D leaves.")
    );

    loop {
        let entry = match editor.readline("lua> ") {
            Ok(entry) => entry,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(err) => return Err(err.into()),
        };
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(entry.as_str());

        if let Some(command) = trimmed.strip_prefix('.') {
            let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
            let arg = arg.trim();
            match name {
                "quit" | "exit" | "q" => break,
                "help" => println!("{HELP}"),
                "doc" => match lua_complete::doc(arg) {
                    Some(doc) => println!("{doc}"),
                    None => println!(
                        "{}",
                        style.red(&format!(
                            "no documentation for {arg:?}: try a function like neori.bind or a \
                             method like Window:title"
                        ))
                    ),
                },
                "signals" => show_result(
                    connection.lua(String::from(
                        "local lines = {} \
                         for _, s in ipairs(neori.signals()) do \
                             lines[#lines + 1] = string.format('%-28s %d  %s', s.name, \
                                 s.handlers or 0, s.description or '(custom)') \
                         end \
                         return table.concat(lines, '\\n')",
                    ))?,
                    style,
                ),
                "load" => match std::fs::read_to_string(arg) {
                    Ok(code) => show_result(connection.lua(code)?, style),
                    Err(err) => println!("{}", style.red(&format!("error reading {arg}: {err}"))),
                },
                "log" => {
                    let on = match arg {
                        "on" => true,
                        "off" => false,
                        _ => !log_shown.load(Ordering::Relaxed),
                    };
                    log_shown.store(on, Ordering::Relaxed);
                    println!("{}", style.dim(if on { "log shown" } else { "log hidden" }));
                }
                "reload" => {
                    let reply =
                        connection.send(Request::Action(Action::LoadConfigFile { path: None }))?;
                    match reply {
                        Ok(_) => {
                            // The Lua state starts over, and so do its variables.
                            if let Some(helper) = editor.helper() {
                                helper.session.borrow_mut().clear();
                            }
                            println!("{}", style.dim("config reloaded; the Lua state is new"));
                        }
                        Err(err) => println!("{}", style.red(&format!("error: {err}"))),
                    }
                }
                _ => println!(
                    "{}",
                    style.red(&format!("unknown command .{name}; .help lists them"))
                ),
            }
            continue;
        }

        let result = connection.lua(globalize_leading_local(&entry))?;
        if result.is_ok() {
            if let Some(helper) = editor.helper() {
                let mut session = helper.session.borrow_mut();
                if !session.is_empty() {
                    session.push('\n');
                }
                session.push_str(&entry);
            }
        }
        show_result(result, style);
    }

    if let Some(path) = &history {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = editor.history_mut().save(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_code_continues() {
        for code in [
            "function f()",
            "for i = 1, 3 do",
            "neori.bind(\"Mod+Q\", function()",
            "local t = {",
            "neori.layout { gaps = 8,",
            "if x then print(1)",
            "x = \"unclosed",
        ] {
            assert!(is_incomplete(code), "{code:?} should continue");
        }
        for code in [
            "1 + 1",
            "neori.focused_window():title()",
            "local x = 1",
            "function f() end",
            // Errors other than an unfinished chunk are shown when it runs.
            "x = = 1",
            "end",
        ] {
            assert!(!is_incomplete(code), "{code:?} is complete");
        }
    }

    #[test]
    fn leading_locals_become_globals() {
        assert_eq!(
            globalize_leading_local("local w = neori.focused_window()"),
            "w = neori.focused_window()"
        );
        assert_eq!(
            globalize_leading_local("  local a, b = 1, 2"),
            "  a, b = 1, 2"
        );
        assert_eq!(
            globalize_leading_local("local function f() return 1 end"),
            "function f() return 1 end"
        );
        assert_eq!(globalize_leading_local("local x"), "x = nil");
        assert_eq!(
            globalize_leading_local("local x <const> = 1"),
            "local x <const> = 1"
        );
        assert_eq!(
            globalize_leading_local("print(1) local y = 2"),
            "print(1) local y = 2"
        );
    }

    #[test]
    fn completion_uses_what_was_typed_before() {
        let helper = ReplHelper {
            session: std::cell::RefCell::new("local w = neori.focused_window()".to_owned()),
            style: Style { on: false },
        };
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, candidates) = helper.complete("print(w:ti", 10, &ctx).unwrap();
        assert_eq!(start, 8);
        let names: Vec<&str> = candidates.iter().map(|c| c.replacement.as_str()).collect();
        assert_eq!(names, ["title"]);

        let (start, candidates) = helper.complete("neori.focused_w", 15, &ctx).unwrap();
        assert_eq!(start, 6);
        let names: Vec<&str> = candidates.iter().map(|c| c.replacement.as_str()).collect();
        assert_eq!(names, ["focused_window", "focused_workspace"]);

        let hint = helper.hint("neori.bind(", 11, &ctx);
        assert_eq!(hint.as_deref(), Some("key, action, options?)"));
    }

    #[test]
    fn log_lines_lose_their_timestamp() {
        let style = Style { on: false };
        assert_eq!(
            format_log_line("[2026-11-27 10:00:00.000Z] hello\tworld", style),
            "│ hello\tworld"
        );
        assert_eq!(
            format_log_line(
                "[2026-11-27 10:00:00.000Z] error: Lua bind callback error",
                style
            ),
            "error: Lua bind callback error"
        );
        assert_eq!(
            format_log_line("stack traceback:", style),
            "│ stack traceback:"
        );
    }
}
