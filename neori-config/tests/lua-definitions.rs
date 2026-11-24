//! Keeps `resources/lua/neori-config.lua`, the LuaLS definitions of the config tables, in sync
//! with the config structs they're generated from.

use std::path::PathBuf;

#[test]
fn config_definitions_are_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/lua/neori-config.lua");
    let generated = neori_config::lua_table::lua_definitions();
    if std::env::var_os("NEORI_UPDATE_LUA_DEFINITIONS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{} is out of date: a config struct changed. Regenerate it with\n  \
         NEORI_UPDATE_LUA_DEFINITIONS=1 cargo test -p neori-config --test lua-definitions",
        path.display()
    );
}

#[test]
fn config_definitions_are_valid_lua() {
    let lua = neori_config::mlua::Lua::new();
    lua.load(neori_config::lua_table::lua_definitions())
        .exec()
        .unwrap();
}
