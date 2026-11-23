//! Completion of Lua code for `neori msg lua`, from the LuaLS type stubs.
//!
//! Works out the type of the expression being typed from the stubs' return types
//! (`neori.focused_window()` is a `neori.Window`, `neori.windows()[1]` too), and offers the names
//! that can follow it. Nothing is evaluated, so pressing Tab never runs any code.

use std::collections::{BTreeSet, HashMap};

const STUBS: &str = include_str!("../../resources/lua/neori.lua");

/// What a name in the stubs is.
#[derive(Debug, Clone, PartialEq)]
enum Type {
    /// The `neori` table.
    Neori,
    /// A class like `neori.Window`, by name without the `neori.` prefix.
    Class(String),
    /// A list of values of a type.
    List(Box<Type>),
    /// A function returning a type.
    Function(Box<Type>),
    /// Anything else.
    Unknown,
}

/// The API, read from the stubs.
struct Api {
    /// Names under `neori.`, with their types.
    neori: HashMap<String, Type>,
    /// Methods (`Class:name`) and fields (`---@field`) of each class.
    members: HashMap<String, HashMap<String, Type>>,
}

/// Parses a stub type like `neori.Window?`, `neori.Window[]` or `string`.
fn parse_type(ty: &str) -> Type {
    let ty = ty.trim().trim_end_matches('?');
    if let Some(item) = ty.strip_suffix("[]") {
        return Type::List(Box::new(parse_type(item)));
    }
    match ty.strip_prefix("neori.") {
        Some(class) if class.chars().all(|c| c.is_alphanumeric() || c == '_') => {
            Type::Class(class.to_owned())
        }
        _ => Type::Unknown,
    }
}

impl Api {
    fn parse() -> Self {
        let mut neori = HashMap::new();
        let mut members: HashMap<String, HashMap<String, Type>> = HashMap::new();
        // The local variable each class is declared on: `local Window = {}`.
        let mut class_vars = HashMap::new();
        let mut current_class = None;
        let mut last_return = Type::Unknown;
        let mut last_type = Type::Unknown;

        for line in STUBS.lines() {
            if let Some(class) = line.strip_prefix("---@class neori.") {
                let class = class.split([':', ' ']).next().unwrap().to_owned();
                current_class = Some(class);
                continue;
            }
            if let Some(field) = line.strip_prefix("---@field ") {
                if let Some(class) = &current_class {
                    let mut parts = field.split_whitespace();
                    let name = parts.next().unwrap_or("").trim_end_matches('?');
                    if !name.starts_with('[') {
                        let ty = parse_type(parts.next().unwrap_or(""));
                        members
                            .entry(class.clone())
                            .or_default()
                            .insert(name.to_owned(), ty);
                    }
                }
                continue;
            }
            if let Some(ty) = line.strip_prefix("---@return ") {
                last_return = parse_type(ty.split_whitespace().next().unwrap_or(""));
                continue;
            }
            if let Some(ty) = line.strip_prefix("---@type ") {
                last_type = parse_type(ty);
                continue;
            }
            if line.starts_with("---") {
                continue;
            }
            if let Some(rest) = line.strip_prefix("local ") {
                if let (Some(var), Some(class)) = (rest.strip_suffix(" = {}"), &current_class) {
                    class_vars.insert(var.to_owned(), class.clone());
                }
            } else if let Some(rest) = line.strip_prefix("function neori.") {
                let name = rest.split('(').next().unwrap();
                let ret = std::mem::replace(&mut last_return, Type::Unknown);
                neori.insert(name.to_owned(), Type::Function(Box::new(ret)));
            } else if let Some(rest) = line.strip_prefix("function ") {
                if let Some((var, method)) = rest.split('(').next().unwrap().split_once(':') {
                    if let Some(class) = class_vars.get(var) {
                        let ret = std::mem::replace(&mut last_return, Type::Unknown);
                        members
                            .entry(class.clone())
                            .or_default()
                            .insert(method.to_owned(), Type::Function(Box::new(ret)));
                    }
                }
            } else if let Some(rest) = line.strip_prefix("neori.") {
                if let Some(name) = rest.strip_suffix(" = {}") {
                    let ty = std::mem::replace(&mut last_type, Type::Unknown);
                    neori.insert(name.to_owned(), ty);
                }
            }
            if !line.is_empty() {
                last_return = Type::Unknown;
                last_type = Type::Unknown;
            }
        }
        Api { neori, members }
    }

    /// The names that can follow `ty` after `separator` (`.` or `:`).
    fn names_after(&self, ty: &Type, separator: char) -> BTreeSet<&str> {
        match (ty, separator) {
            (Type::Neori, '.') => self.neori.keys().map(String::as_str).collect(),
            (Type::Class(class), _) => self
                .members
                .get(class)
                .into_iter()
                .flatten()
                // Methods follow `:`, fields `.`.
                .filter(|(_, ty)| matches!(ty, Type::Function(_)) == (separator == ':'))
                .map(|(name, _)| name.as_str())
                .collect(),
            _ => BTreeSet::new(),
        }
    }

    /// The type of `name` after `separator` on a value of type `ty`.
    fn member(&self, ty: &Type, separator: char, name: &str) -> Type {
        match (ty, separator) {
            (Type::Neori, '.') => self.neori.get(name).cloned().unwrap_or(Type::Unknown),
            (Type::Class(class), _) => self
                .members
                .get(class)
                .and_then(|m| m.get(name))
                .cloned()
                .unwrap_or(Type::Unknown),
            _ => Type::Unknown,
        }
    }
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Finds the end of the bracket that opens at `start` in `chars`, or `None` if it isn't closed.
fn closing(chars: &[char], start: usize) -> Option<usize> {
    let (open, close) = match chars[start] {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        _ => return None,
    };
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(start) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Completes the Lua code `text`: returns the candidates for the whole text, each one `text`
/// with its last name completed. `globals` are the global names to offer at the top level.
pub fn complete(text: &str, globals: &[String]) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();

    // The name being typed, at the end.
    let mut partial_start = chars.len();
    while partial_start > 0 && is_name_char(chars[partial_start - 1]) {
        partial_start -= 1;
    }
    let partial: String = chars[partial_start..].iter().collect();
    let before: String = chars[..partial_start].iter().collect();

    let names: BTreeSet<String> = match chars[..partial_start].last() {
        Some(&separator @ ('.' | ':')) => {
            let api = Api::parse();
            let ty = chain_type(&api, &chars[..partial_start - 1]);
            api.names_after(&ty, separator)
                .into_iter()
                .map(str::to_owned)
                .collect()
        }
        _ => {
            let mut names: BTreeSet<String> = globals.iter().cloned().collect();
            names.insert("neori".to_owned());
            names
        }
    };

    names
        .into_iter()
        .filter(|name| name.starts_with(&partial))
        .map(|name| format!("{before}{name}"))
        .collect()
}

/// The type of the expression that ends `chars`, like `neori.windows()[1]`, or `Unknown`.
fn chain_type(api: &Api, chars: &[char]) -> Type {
    // Find where the chain starts: go back over names, `.`, `:` and balanced brackets.
    let mut start = chars.len();
    loop {
        match start.checked_sub(1).map(|i| chars[i]) {
            Some(c) if is_name_char(c) || c == '.' || c == ':' => start -= 1,
            Some(')' | ']') => {
                let close = chars[start - 1];
                let open = if close == ')' { '(' } else { '[' };
                let mut depth = 0;
                let mut i = start;
                loop {
                    if i == 0 {
                        return Type::Unknown;
                    }
                    i -= 1;
                    if chars[i] == close {
                        depth += 1;
                    } else if chars[i] == open {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                start = i;
            }
            _ => break,
        }
    }

    // Walk the chain forwards.
    let chain = &chars[start..];
    let mut i = 0;
    let mut name = String::new();
    while i < chain.len() && is_name_char(chain[i]) {
        name.push(chain[i]);
        i += 1;
    }
    let mut ty = if name == "neori" {
        Type::Neori
    } else {
        Type::Unknown
    };
    while i < chain.len() {
        match chain[i] {
            separator @ ('.' | ':') => {
                i += 1;
                let mut name = String::new();
                while i < chain.len() && is_name_char(chain[i]) {
                    name.push(chain[i]);
                    i += 1;
                }
                ty = api.member(&ty, separator, &name);
            }
            '(' => {
                let Some(end) = closing(chain, i) else {
                    return Type::Unknown;
                };
                ty = match ty {
                    Type::Function(ret) => *ret,
                    _ => Type::Unknown,
                };
                i = end + 1;
            }
            '[' => {
                let Some(end) = closing(chain, i) else {
                    return Type::Unknown;
                };
                ty = match ty {
                    Type::List(item) => *item,
                    _ => Type::Unknown,
                };
                i = end + 1;
            }
            _ => return Type::Unknown,
        }
    }
    ty
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_names(text: &str) -> Vec<String> {
        complete(text, &["my_helper".to_owned(), "neori".to_owned()])
    }

    #[test]
    fn completes_neori_names() {
        assert_eq!(
            complete_names("neori.focused_"),
            [
                "neori.focused_output",
                "neori.focused_window",
                "neori.focused_workspace"
            ]
        );
        // Namespaces too.
        assert!(complete_names("neori.").contains(&"neori.misc".to_owned()));
        assert!(complete_names("neori.").contains(&"neori.layout".to_owned()));
    }

    #[test]
    fn completes_methods_from_return_types() {
        assert!(complete_names("neori.focused_window():ti")
            .contains(&"neori.focused_window():title".to_owned()));
        // A list, indexed.
        assert!(complete_names("neori.windows()[1]:app")
            .contains(&"neori.windows()[1]:app_id".to_owned()));
        // A method returning another class.
        assert!(complete_names("neori.focused_window():geometry().w")
            .contains(&"neori.focused_window():geometry().width".to_owned()));
        assert_eq!(
            complete_names("neori.focused_output():work"),
            ["neori.focused_output():workarea"]
        );
        // Arguments with brackets of their own.
        assert!(complete_names("neori.window_at(f(1), 2):ti")
            .contains(&"neori.window_at(f(1), 2):title".to_owned()));
    }

    #[test]
    fn methods_after_colon_fields_after_dot() {
        let methods = complete_names("neori.focused_window():");
        assert!(methods.contains(&"neori.focused_window():close".to_owned()));
        assert!(!complete_names("neori.focused_window().")
            .contains(&"neori.focused_window().close".to_owned()));
        assert_eq!(complete_names("neori.misc.p"), ["neori.misc.prefer_no_csd"]);
    }

    #[test]
    fn completes_globals_at_the_top_level_and_inside_code() {
        assert_eq!(complete_names("my_"), ["my_helper"]);
        assert_eq!(complete_names("ne"), ["neori"]);
        assert_eq!(
            complete_names("print(neori.focused_window():ti"),
            ["print(neori.focused_window():title"]
        );
        assert_eq!(complete_names("x = 1; my"), ["x = 1; my_helper"]);
    }

    #[test]
    fn unknown_types_complete_nothing() {
        assert!(complete_names("foo.").is_empty());
        assert!(complete_names("neori.nope():").is_empty());
        assert!(complete_names("neori.focused_window(:").is_empty());
    }
}
