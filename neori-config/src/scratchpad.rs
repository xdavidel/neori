use neori_config_derive::LuaTable;

/// A named scratchpad declared in the config.
///
/// Scratchpads are addressed by name from the scratchpad actions
/// (`toggle-scratchpad term`, …). The empty name is the default scratchpad, used when an
/// action carries no name.
///
/// A `spawn` command makes the scratchpad self-populating: the first time it is toggled while
/// empty, the command is run and the new window is adopted into the scratchpad.
#[derive(Debug, Clone, PartialEq, Eq, Default, LuaTable)]
pub struct Scratchpad {
    /// Scratchpad name; empty string for the default scratchpad.
    pub name: String,
    /// Command (argv) to spawn when toggling this scratchpad while empty.
    #[lua(with = crate::parse_lua::parse_scratchpad_spawn, lua_type = "string|string[]")]
    pub spawn: Option<Vec<String>>,
    /// Shell command to spawn when toggling this scratchpad while empty.
    pub spawn_sh: Option<String>,
}

impl Scratchpad {
    /// Returns the spawn command as an `Action`, if one is configured.
    pub fn spawn_action(&self) -> Option<crate::Action> {
        if let Some(command) = &self.spawn {
            return Some(crate::Action::Spawn {
                command: command.clone(),
            });
        }
        if let Some(command) = &self.spawn_sh {
            return Some(crate::Action::SpawnSh {
                command: command.clone(),
            });
        }
        None
    }
}
