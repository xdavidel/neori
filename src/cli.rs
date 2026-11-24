use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use neori_ipc::{Action, OutputAction};

use crate::utils::version;

#[derive(Parser)]
#[command(author, version = version(), about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
#[command(subcommand_value_name = "SUBCOMMAND")]
#[command(subcommand_help_heading = "Subcommands")]
pub struct Cli {
    /// Path to config file (default: `$XDG_CONFIG_HOME/neori/config.lua`).
    ///
    /// This can also be set with the `NEORI_CONFIG` environment variable. If both are set, the
    /// command line argument takes precedence.
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Import environment globally to systemd and D-Bus, run D-Bus services.
    ///
    /// Set this flag in a systemd service started by your display manager, or when running
    /// manually as your main compositor instance. Do not set when running as a nested window, or
    /// on a TTY as your non-main compositor instance, to avoid messing up the global environment.
    #[arg(long)]
    pub session: bool,
    /// Command to run upon compositor startup.
    #[arg(last = true)]
    pub command: Vec<OsString>,

    #[command(subcommand)]
    pub subcommand: Option<Sub>,
}

/// Fixes the zsh completion script that clap generates.
///
/// clap lists the startup command (`neori -- <command>`) as a normal optional first positional
/// argument, although it's only accepted after `--`. zsh then puts the subcommand into it, and
/// completes nothing under `neori msg`. Drop it from the script.
pub fn fix_zsh_completion(script: &str) -> String {
    let script: String = script
        .lines()
        .filter(|line| !line.starts_with("'::command -- "))
        .map(|line| format!("{line}\n"))
        .collect();
    // The Lua code of `neori msg lua` completes through `neori msg lua --complete`.
    let script = script.replacen(
        "'::code -- The Lua code to run:_default' \\",
        "'::code -- The Lua code to run:_neori_lua_code' \\",
        1,
    );
    let script = script.replacen(
        "if [ \"$funcstack[1]\" = \"_neori\" ]; then",
        concat!(
            "(( $+functions[_neori_lua_code] )) ||\n",
            "_neori_lua_code() {\n",
            "    setopt localoptions extendedglob\n",
            "    local -a candidates\n",
            "    candidates=(${(f)\"$(neori msg lua --complete \"$PREFIX\" 2>/dev/null)\"})\n",
            "    (( $#candidates )) || return 1\n",
            "    # Offer the names, with the code before them as a prefix, so lists show `title`\n",
            "    # rather than the whole code.\n",
            "    local head=${PREFIX%%[[:IDENT:]]#}\n",
            "    compadd -S '' -p \"$head\" -- ${candidates[@]#\"$head\"}\n",
            "}\n",
            "\n",
            "if [ \"$funcstack[1]\" = \"_neori\" ]; then",
        ),
        1,
    );
    // The subcommand is now the first positional argument of `neori` instead of the second. Only
    // the top-level block changes: `neori msg output <name> <action>` has its own `$line[2]`.
    let top_level = [
        (
            "words=($line[2] \"${words[@]}\")",
            "words=($line[1] \"${words[@]}\")",
        ),
        (":neori-command-$line[2]:", ":neori-command-$line[1]:"),
        ("case $line[2] in", "case $line[1] in"),
    ];
    top_level
        .into_iter()
        .fold(script, |script, (from, to)| script.replacen(from, to, 1))
}

#[derive(Subcommand)]
pub enum Sub {
    /// Communicate with the running neori instance.
    Msg {
        #[command(subcommand)]
        msg: Msg,
        /// Format output as JSON.
        #[arg(short, long)]
        json: bool,
        /// Print the IPC request as JSON instead of sending it.
        #[arg(long)]
        print_request: bool,
    },
    /// Validate the config file.
    Validate {
        /// Path to config file (default: `$XDG_CONFIG_HOME/neori/config.lua`).
        ///
        /// This can also be set with the `NEORI_CONFIG` environment variable. If both are set, the
        /// command line argument takes precedence.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Cause a panic to check if the backtraces are good.
    Panic,
    /// Generate shell completions.
    Completions { shell: CompletionShell },
}

#[derive(Subcommand)]
pub enum Msg {
    /// List connected outputs.
    Outputs,
    /// List workspaces.
    Workspaces,
    /// List open windows.
    Windows,
    /// List open layer-shell surfaces.
    Layers,
    /// Get the configured keyboard layouts.
    KeyboardLayouts,
    /// Print information about the focused output.
    FocusedOutput,
    /// Print information about the focused window.
    FocusedWindow,
    /// Pick a window with the mouse and print information about it.
    PickWindow,
    /// Pick a color from the screen with the mouse.
    PickColor,
    /// Perform an action.
    Action {
        #[command(subcommand)]
        action: Action,
    },
    /// Change output configuration temporarily.
    ///
    /// The configuration is changed temporarily and not saved into the config file. If the output
    /// configuration subsequently changes in the config file, these temporary changes will be
    /// forgotten.
    Output {
        /// Output name.
        ///
        /// Run `neori msg outputs` to see the output names.
        #[arg()]
        output: String,
        /// Configuration to apply.
        #[command(subcommand)]
        action: OutputAction,
    },
    /// Create a virtual output.
    CreateVirtualOutput {
        /// Optional name for the created output.
        ///
        /// When omitted, a unique `HEADLESS-N` name is generated.
        #[arg(long)]
        name: Option<String>,
        /// Width in pixels.
        #[arg(long, default_value = "1920")]
        width: u16,
        /// Height in pixels.
        #[arg(long, default_value = "1080")]
        height: u16,
        /// Refresh rate in Hz.
        #[arg(long, default_value = "60")]
        refresh_rate: u32,
    },
    /// Remove a virtual output.
    RemoveVirtualOutput {
        /// Identifier of the output to remove.
        #[arg()]
        name: String,
    },
    /// Start continuously receiving events from the compositor.
    EventStream,
    /// Print the version of the running neori instance.
    Version,
    /// Request an error from the running neori instance.
    RequestError,
    /// Print the overview state.
    OverviewState,
    /// List screencasts.
    Casts,
    /// Send a raw JSON request to the compositor, reading from stdin.
    RawRequest,
    /// Run Lua code in the compositor, in the config's Lua state.
    ///
    /// An expression prints its value: `neori msg lua 'neori.focused_window():title()'`.
    /// Statements can `return` values. Changes apply right away.
    Lua {
        /// The Lua code to run.
        #[arg(required_unless_present_any = ["file", "complete"], conflicts_with_all = ["file", "complete"])]
        code: Option<String>,
        /// Run the Lua code in this file instead.
        #[arg(short, long, conflicts_with = "complete")]
        file: Option<std::path::PathBuf>,
        /// Print the completions of this Lua code, one per line, for shell completion.
        #[arg(long, hide = true)]
        complete: Option<String>,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
    Nushell,
}

impl TryFrom<CompletionShell> for Shell {
    type Error = &'static str;

    fn try_from(shell: CompletionShell) -> Result<Self, Self::Error> {
        match shell {
            CompletionShell::Bash => Ok(Shell::Bash),
            CompletionShell::Elvish => Ok(Shell::Elvish),
            CompletionShell::Fish => Ok(Shell::Fish),
            CompletionShell::PowerShell => Ok(Shell::PowerShell),
            CompletionShell::Zsh => Ok(Shell::Zsh),
            CompletionShell::Nushell => Err("Nushell should be handled separately"),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    fn zsh_completion() -> String {
        // Generating the script for the whole CLI needs more stack than a test thread has.
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut script = Vec::new();
                clap_complete::generate(
                    clap_complete::Shell::Zsh,
                    &mut Cli::command(),
                    "neori",
                    &mut script,
                );
                fix_zsh_completion(&String::from_utf8(script).unwrap())
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn zsh_completion_completes_subcommands() {
        let script = zsh_completion();
        // The startup command no longer takes the subcommand's place.
        assert!(!script.contains("Command to run upon compositor startup"));
        let top = script.split("(msg)").next().unwrap();
        assert!(top.contains("case $line[1] in"), "{top}");
        assert!(!top.contains("$line[2]"), "{top}");
        // `neori msg output <name> <action>` keeps its second positional.
        assert!(script.contains(":neori-msg-output-command-$line[2]:"));
    }

    #[test]
    fn zsh_completion_completes_lua_code() {
        let script = zsh_completion();
        assert!(script.contains("'::code -- The Lua code to run:_neori_lua_code' \\"));
        let function = script.find("_neori_lua_code() {").unwrap();
        let dispatch = script
            .find("if [ \"$funcstack[1]\" = \"_neori\" ]; then")
            .unwrap();
        assert!(
            function < dispatch,
            "the function must be defined before it's used"
        );
        assert!(script.contains("neori msg lua --complete \"$PREFIX\""));
    }
}
