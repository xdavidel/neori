//! `neori.util`: table, string, math and geometry helpers ported from awesome's `gears`, plus
//! `neori.util.fs`, which follows `gears.filesystem` but is written in Rust because the original
//! needs GLib.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use mlua::prelude::*;

const MATH_LUA: &str = include_str!("lua_util/math.lua");
const TABLE_LUA: &str = include_str!("lua_util/table.lua");
const STRING_LUA: &str = include_str!("lua_util/string.lua");
const GEOMETRY_LUA: &str = include_str!("lua_util/geometry.lua");
const DUMP_LUA: &str = include_str!("lua_util/dump.lua");

/// Builds the `neori.util` table. `config_dir` is the directory of the config file.
pub fn load_util(lua: &Lua, config_dir: &Path) -> LuaResult<LuaTable> {
    let load = |code: &str, name: &str, args: LuaMultiValue| -> LuaResult<LuaValue> {
        lua.load(code)
            .set_name(format!("neori.util.{name}"))
            .call(args)
    };
    let util = lua.create_table()?;

    let math = load(MATH_LUA, "math", LuaMultiValue::new())?;
    let table = load(TABLE_LUA, "table", LuaMultiValue::from_iter([math.clone()]))?;
    util.set("math", math)?;
    util.set("table", table)?;
    util.set("string", load(STRING_LUA, "string", LuaMultiValue::new())?)?;
    util.set(
        "geometry",
        load(GEOMETRY_LUA, "geometry", LuaMultiValue::new())?,
    )?;
    util.set("dump", load(DUMP_LUA, "dump", LuaMultiValue::new())?)?;
    util.set("fs", fs_table(lua, config_dir)?)?;
    Ok(util)
}

/// `$name` if it's set and not empty.
fn env_dir(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// A directory with a trailing slash, like `gears.filesystem` returns them, so that file names
/// can be appended with `..`.
fn with_slash(dir: impl Into<String>) -> String {
    let mut dir = dir.into();
    if !dir.ends_with('/') {
        dir.push('/');
    }
    dir
}

fn xdg_home(var: &str, fallback: &str) -> String {
    with_slash(env_dir(var).unwrap_or_else(|| format!("{}/{fallback}", home())))
}

/// Whether the current user may access `path` in `mode` (`libc::R_OK`, …), like `access(2)`.
fn can_access(path: &str, mode: libc::c_int) -> bool {
    let Ok(path) = CString::new(path) else {
        return false;
    };
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call.
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Whether `path` exists and isn't a directory.
fn is_file(path: &str) -> bool {
    Path::new(path).metadata().is_ok_and(|m| !m.is_dir())
}

fn fs_table(lua: &Lua, config_dir: &Path) -> LuaResult<LuaTable> {
    let fs = lua.create_table()?;

    fs.set(
        "xdg_config_home",
        lua.create_function(|_, ()| Ok(xdg_home("XDG_CONFIG_HOME", ".config")))?,
    )?;
    fs.set(
        "xdg_cache_home",
        lua.create_function(|_, ()| Ok(xdg_home("XDG_CACHE_HOME", ".cache")))?,
    )?;
    fs.set(
        "xdg_data_home",
        lua.create_function(|_, ()| Ok(xdg_home("XDG_DATA_HOME", ".local/share")))?,
    )?;
    fs.set(
        "xdg_data_dirs",
        lua.create_function(|_, ()| {
            let dirs = env_dir("XDG_DATA_DIRS")
                .unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
            Ok(dirs
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(with_slash)
                .collect::<Vec<_>>())
        })?,
    )?;

    let config_dir = with_slash(config_dir.to_string_lossy());
    fs.set(
        "config_dir",
        lua.create_function(move |_, ()| Ok(config_dir.clone()))?,
    )?;
    fs.set(
        "cache_dir",
        lua.create_function(|_, ()| {
            let dir = xdg_home("XDG_CACHE_HOME", ".cache") + "neori/";
            // Like gears: a directory that can't be made shows up when it's written to.
            let _ = std::fs::create_dir_all(&dir);
            Ok(dir)
        })?,
    )?;

    fs.set(
        "file_readable",
        lua.create_function(|_, path: String| Ok(is_file(&path) && can_access(&path, libc::R_OK)))?,
    )?;
    fs.set(
        "file_executable",
        lua.create_function(|_, path: String| Ok(is_file(&path) && can_access(&path, libc::X_OK)))?,
    )?;
    fs.set(
        "dir_readable",
        lua.create_function(|_, path: String| Ok(is_dir(&path) && can_access(&path, libc::R_OK)))?,
    )?;
    fs.set(
        "dir_writable",
        lua.create_function(|_, path: String| Ok(is_dir(&path) && can_access(&path, libc::W_OK)))?,
    )?;
    fs.set(
        "is_dir",
        lua.create_function(|_, path: String| Ok(is_dir(&path)))?,
    )?;
    fs.set(
        "mkdir",
        lua.create_function(
            |_, dir: String| match std::fs::create_dir_all(PathBuf::from(&dir)) {
                Ok(()) => Ok((true, None)),
                Err(err) => Ok((false, Some(format!("{dir}: {err}")))),
            },
        )?,
    )?;

    Ok(fs)
}
