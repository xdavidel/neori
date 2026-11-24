//! Keeps the LuaLS type stubs in `resources/lua/neori.lua` in sync with the real Lua API.
//!
//! Every function and namespace of the `neori` table, every method of the `Window`, `Output`,
//! `Workspace`, `Device` and `Timer` userdata, the built-in signals and the bind options must be
//! declared in the stubs, and the stubs must not declare anything that doesn't exist.

use std::collections::BTreeSet;
use std::path::Path;

use neori_config::parse_lua::{BIND_OPTIONS, CONFIG_SECTIONS};
use neori_config::signals::BUILTIN_SIGNALS;
use neori_config::{mlua, Config};
use regex::Regex;

const STUBS: &str = include_str!("../../resources/lua/neori.lua");
const PARSE_LUA: &str = include_str!("../src/parse_lua.rs");

fn set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_owned).collect()
}

/// Asserts that two sets of names are equal, naming what's missing on each side.
fn assert_same(what: &str, stubs: &BTreeSet<String>, api: &BTreeSet<String>) {
    let missing: Vec<_> = api.difference(stubs).collect();
    let extra: Vec<_> = stubs.difference(api).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: missing from the stubs: {missing:?}; in the stubs but not in the API: {extra:?}"
    );
}

/// The lines of the doc block that starts with `header`, up to the next line that isn't a
/// `---` annotation.
fn annotation_block<'a>(header: &str) -> Vec<&'a str> {
    let mut lines = STUBS.lines().skip_while(|l| !l.starts_with(header));
    assert!(lines.next().is_some(), "{header} not found in the stubs");
    lines.take_while(|l| l.starts_with("---")).collect()
}

#[test]
fn stubs_are_valid_lua() {
    let lua = mlua::Lua::new();
    lua.load(STUBS)
        .set_name("neori.lua")
        .exec()
        .expect("the stubs must load as Lua");
}

#[test]
fn stubs_declare_every_neori_function_and_namespace() {
    let (_config, host) = Config::parse_with_host(
        Path::new("test.lua"),
        r#"
        functions, namespaces = {}, {}
        for name, value in pairs(neori) do
            if type(value) == "function" then
                functions[#functions + 1] = name
            elseif type(value) == "table" then
                namespaces[#namespaces + 1] = name
            end
        end
        "#,
    )
    .config
    .unwrap();
    let names = |global: &str| -> BTreeSet<String> {
        let mlua::Value::Table(list) = host.read_global(global).unwrap() else {
            panic!("{global} is not a table");
        };
        list.sequence_values::<String>()
            .map(Result::unwrap)
            .collect()
    };
    let declared = |pattern: &str| -> BTreeSet<String> {
        let re = Regex::new(pattern).unwrap();
        re.captures_iter(STUBS).map(|c| c[1].to_owned()).collect()
    };

    assert_same(
        "neori functions",
        &declared(r"(?m)^function neori\.(\w+)\("),
        &names("functions"),
    );
    // Config sections and neori.misc, declared as `neori.<name> = {}`.
    assert_same(
        "neori namespaces",
        &declared(r"(?m)^neori\.(\w+) = \{\}"),
        &names("namespaces"),
    );
    let sections: BTreeSet<String> = CONFIG_SECTIONS.iter().map(|s| s.to_string()).collect();
    let mut expected = sections.clone();
    expected.insert("misc".to_owned());
    expected.insert("rules".to_owned());
    assert_eq!(names("namespaces"), expected);
}

#[test]
fn stubs_declare_every_method() {
    // The Rust type of each userdata, and its class in the stubs.
    let classes = [
        ("Window", "Window"),
        ("LuaOutput", "Output"),
        ("LuaWorkspace", "Workspace"),
        ("LuaDevice", "Device"),
        ("LuaTimer", "Timer"),
        ("LuaLayer", "LayerSurface"),
    ];
    let method = Regex::new(r#"add_(?:method|function)\(\s*"(\w+)""#).unwrap();

    for (rust_type, class) in classes {
        // The methods registered in `impl mlua::UserData for <rust_type>`.
        let header = format!("impl mlua::UserData for {rust_type} {{");
        let start = PARSE_LUA
            .find(&header)
            .unwrap_or_else(|| panic!("{header} not found"));
        let body = &PARSE_LUA[start..];
        let end = body.find("\n}\n").unwrap();
        let body = &body[..end];
        let mut api: BTreeSet<String> = method
            .captures_iter(body)
            .map(|c| c[1].to_owned())
            .collect();
        if body.contains("add_signal_methods(") {
            api.extend(set(["connect", "disconnect", "emit", "signals"]));
        }

        assert!(
            STUBS.contains(&format!("---@class neori.{class}\nlocal {class} = {{}}")),
            "the stubs must declare `---@class neori.{class}` followed by `local {class} = {{}}`"
        );
        let re = Regex::new(&format!(r"(?m)^function {class}:(\w+)\(")).unwrap();
        let stubs: BTreeSet<String> = re.captures_iter(STUBS).map(|c| c[1].to_owned()).collect();
        assert_same(&format!("{class} methods"), &stubs, &api);
    }
}

#[test]
fn stubs_list_every_builtin_signal() {
    let builtin: Vec<&str> = BUILTIN_SIGNALS.iter().map(|s| s.name).collect();

    // The alias, in the same order as the built-in table.
    let alias: Vec<String> = annotation_block("---@alias neori.Signal")
        .iter()
        .filter_map(|l| l.strip_prefix("---| \""))
        .map(|l| l.split('"').next().unwrap().to_owned())
        .collect();
    assert_eq!(alias, builtin, "neori.Signal");

    // The overloads of neori.on and neori.connect, which type each signal's handler.
    for (function, param) in [("on", "event"), ("connect", "signal")] {
        let start = STUBS.find(&format!("function neori.{function}(")).unwrap();
        let doc_start = STUBS[..start].rfind("\n\n").unwrap();
        let re = Regex::new(&format!(r#"---@overload fun\({param}: "([\w-]+)""#)).unwrap();
        let overloads: Vec<String> = re
            .captures_iter(&STUBS[doc_start..start])
            .map(|c| c[1].to_owned())
            .collect();
        assert_eq!(overloads, builtin, "overloads of neori.{function}");
    }
}

#[test]
fn stubs_list_every_bind_option() {
    let fields: BTreeSet<String> = annotation_block("---@class neori.BindOptions")
        .iter()
        .filter_map(|l| l.strip_prefix("---@field "))
        .map(|l| l.split('?').next().unwrap().to_owned())
        .collect();
    assert_same(
        "neori.BindOptions",
        &fields,
        &set(BIND_OPTIONS.iter().copied()),
    );
}

/// The `---@field` names of the class `class` in the stubs.
fn class_fields(class: &str) -> BTreeSet<String> {
    annotation_block(&format!("---@class {class}"))
        .iter()
        .filter_map(|l| l.strip_prefix("---@field "))
        .map(|l| l.split([' ', '?']).next().unwrap().to_owned())
        .collect()
}

#[test]
fn stubs_declare_neori_rules() {
    let lua = mlua::Lua::new();
    let neori = lua.create_table().unwrap();
    let (rules, classes) = neori_config::parse_lua::load_rules(&lua, &neori).unwrap();

    let functions: BTreeSet<String> = rules
        .pairs::<String, mlua::Value>()
        .map(|pair| pair.unwrap().0)
        .collect();
    let re = Regex::new(r"(?m)^function neori\.rules\.(\w+)\(").unwrap();
    let declared: BTreeSet<String> = re.captures_iter(STUBS).map(|c| c[1].to_owned()).collect();
    assert_same("neori.rules functions", &declared, &functions);

    // Every method a rule calls exists in the stubs of its class, and the rule tables list every
    // key of every class.
    let mut get_keys = BTreeSet::new();
    let mut set_keys = BTreeSet::new();
    for pair in classes.pairs::<String, mlua::Table>() {
        let (class, properties) = pair.unwrap();
        let re = Regex::new(&format!(r"(?m)^function {class}:(\w+)\(")).unwrap();
        let methods: BTreeSet<String> = re.captures_iter(STUBS).map(|c| c[1].to_owned()).collect();
        assert!(!methods.is_empty(), "no methods of {class} in the stubs");
        for kind in ["get", "set", "open"] {
            let Ok(table) = properties.get::<mlua::Table>(kind) else {
                continue;
            };
            let keys = if kind == "get" {
                &mut get_keys
            } else {
                &mut set_keys
            };
            for pair in table.pairs::<String, String>() {
                let (key, method) = pair.unwrap();
                assert!(
                    methods.contains(&method),
                    "{class}.{kind}.{key} calls {class}:{method}, which isn't in the stubs"
                );
                keys.insert(key);
            }
        }
    }
    assert_same(
        "neori.RuleWhere",
        &class_fields("neori.RuleWhere"),
        &get_keys,
    );
    assert_same("neori.RuleSet", &class_fields("neori.RuleSet"), &set_keys);
}
