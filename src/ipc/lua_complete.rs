//! Completion of Lua code for `neori msg lua`, from the LuaLS type stubs.
//!
//! Works out the type of the expression being typed from the stubs' types
//! (`neori.focused_window()` is a `neori.Window`, `neori.windows()[1]` too), and offers the names
//! that can follow it. It also follows the code typed before it: variables assigned from typed
//! expressions (`local w = neori.focused_window()`), loop variables (`for _, w in
//! ipairs(neori.windows())`) and event handler parameters (`neori.on("window-opened",
//! function(w)`), and the keys of config tables (`neori.layout { gaps = 8, focus_ring = { … } }`).
//! Nothing is evaluated, so pressing Tab never runs any code; the keys of other tables, like
//! `string` or the config's own, come from the caller.

use std::collections::{BTreeSet, HashMap};

const STUBS: &str = concat!(
    include_str!("../../resources/lua/neori.lua"),
    "\n",
    include_str!("../../resources/lua/neori-config.lua"),
);

/// What a name in the stubs is.
#[derive(Debug, Clone, PartialEq)]
enum Type {
    /// The `neori` table.
    Neori,
    /// A class like `neori.Window`, by name without the `neori.` prefix.
    Class(String),
    /// A namespace like `neori.rules`, whose functions are called with `.`, by its class.
    Namespace(String),
    /// A list of values of a type.
    List(Box<Type>),
    /// A function: what it returns, and the parameter types of each way to call it.
    Function {
        ret: Box<Type>,
        signatures: Vec<Vec<Type>>,
    },
    /// Anything else.
    Unknown,
}

/// The API, read from the stubs.
struct Api {
    /// Names under `neori.`, with their types.
    neori: HashMap<String, Type>,
    /// Methods (`Class:name`) and fields (`---@field`) of each class.
    members: HashMap<String, HashMap<String, Type>>,
    /// The parameters event handlers get, by event name, from `neori.on`'s overloads.
    events: HashMap<String, Vec<Type>>,
}

/// Splits `s` at `separator` where it isn't inside brackets.
fn split_top_level(s: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            c if c == separator && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => (),
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parses a stub type like `neori.Window?`, `neori.Window[]`,
/// `neori.config.Match|neori.config.Match[]` or `string`. Of a union, the first alternative
/// that's known wins.
fn parse_type(ty: &str) -> Type {
    let ty = ty.trim().trim_end_matches('?');
    let alternatives = split_top_level(ty, '|');
    if alternatives.len() > 1 {
        return alternatives
            .into_iter()
            .map(parse_type)
            .find(|ty| *ty != Type::Unknown)
            .unwrap_or(Type::Unknown);
    }
    if let Some(item) = ty.strip_suffix("[]") {
        let item = item
            .strip_prefix('(')
            .and_then(|item| item.strip_suffix(')'))
            .unwrap_or(item);
        return Type::List(Box::new(parse_type(item)));
    }
    if let Some(rest) = ty.strip_prefix("fun(") {
        // The parameters end at the `)` that closes `fun(`.
        let mut depth = 1;
        let end = rest
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => (),
                }
                depth == 0
            })
            .map_or(rest.len(), |(i, _)| i);
        return Type::Function {
            ret: Box::new(Type::Unknown),
            signatures: vec![parse_params(&rest[..end])],
        };
    }
    match ty.strip_prefix("neori.") {
        // `neori.Window`, or `neori.config.Layout` for a config table.
        Some(class)
            if class
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.') =>
        {
            Type::Class(class.to_owned())
        }
        _ => Type::Unknown,
    }
}

/// Parses the parameters of a `fun(a: T, b: U)` type: `a: T, b: U`.
fn parse_params(params: &str) -> Vec<Type> {
    if params.trim().is_empty() {
        return Vec::new();
    }
    split_top_level(params, ',')
        .into_iter()
        .map(|param| parse_type(param.split_once(':').map_or("", |(_, ty)| ty)))
        .collect()
}

/// The event name of an overload like `fun(event: "window-opened", handler: …)`.
fn event_name(overload: &str) -> Option<String> {
    let rest = overload.strip_prefix("fun(")?;
    let (_, rest) = rest.split_once(':')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let (name, _) = rest.split_once('"')?;
    Some(name.to_owned())
}

/// The class of the namespace at `path` under `neori` (`util.table`), if it is one.
fn namespace_class(
    neori: &HashMap<String, Type>,
    members: &HashMap<String, HashMap<String, Type>>,
    path: &str,
) -> Option<String> {
    let mut parts = path.split('.');
    let mut ty = neori.get(parts.next()?)?;
    for part in parts {
        let Type::Namespace(class) = ty else {
            return None;
        };
        ty = members.get(class)?.get(part)?;
    }
    match ty {
        Type::Namespace(class) => Some(class.clone()),
        _ => None,
    }
}

impl Api {
    fn parse() -> Self {
        let mut neori = HashMap::new();
        let mut members: HashMap<String, HashMap<String, Type>> = HashMap::new();
        let mut events = HashMap::new();
        // The local variable each class is declared on: `local Window = {}`.
        let mut class_vars = HashMap::new();
        let mut current_class = None;
        // The class declared just above, for a namespace like `neori.rules = {}`.
        let mut class_above: Option<String> = None;
        let mut last_return = Type::Unknown;
        let mut last_type = Type::Unknown;
        // The `---@param` and `---@overload` annotations of the function that follows.
        let mut params = Vec::new();
        let mut overloads = Vec::new();

        for line in STUBS.lines() {
            if line.is_empty() {
                class_above = None;
            }
            if let Some(class) = line.strip_prefix("---@class neori.") {
                let class = class.split([':', ' ']).next().unwrap().to_owned();
                current_class = Some(class.clone());
                class_above = Some(class);
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
            if let Some(param) = line.strip_prefix("---@param ") {
                params.push(parse_type(param.split_whitespace().nth(1).unwrap_or("")));
                continue;
            }
            if let Some(overload) = line.strip_prefix("---@overload ") {
                if let Type::Function { mut signatures, .. } = parse_type(overload) {
                    let signature = signatures.pop().unwrap_or_default();
                    // `fun(event: "window-opened", handler: fun(window: neori.Window))`.
                    if let Some(event) = event_name(overload) {
                        if let Some(Type::Function { signatures, .. }) = signature.get(1) {
                            let handler = signatures.first().cloned().unwrap_or_default();
                            events.insert(event, handler);
                        }
                    }
                    overloads.push(signature);
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
            let mut function = |ret: Type| {
                let mut signatures = vec![std::mem::take(&mut params)];
                signatures.append(&mut overloads);
                Type::Function {
                    ret: Box::new(ret),
                    signatures,
                }
            };
            if let Some(rest) = line.strip_prefix("local ") {
                if let (Some(var), Some(class)) = (rest.strip_suffix(" = {}"), &current_class) {
                    class_vars.insert(var.to_owned(), class.clone());
                }
            } else if let Some(rest) = line.strip_prefix("function neori.") {
                let name = rest.split('(').next().unwrap();
                let ret = std::mem::replace(&mut last_return, Type::Unknown);
                // `function neori.rules.add(…)`: a function of a namespace.
                if let Some((namespace, name)) = name.rsplit_once('.') {
                    if let Some(class) = namespace_class(&neori, &members, namespace) {
                        members
                            .entry(class)
                            .or_default()
                            .insert(name.to_owned(), function(ret));
                    }
                } else {
                    neori.insert(name.to_owned(), function(ret));
                }
            } else if let Some(rest) = line.strip_prefix("function ") {
                if let Some((var, method)) = rest.split('(').next().unwrap().split_once(':') {
                    if let Some(class) = class_vars.get(var) {
                        let ret = std::mem::replace(&mut last_return, Type::Unknown);
                        members
                            .entry(class.clone())
                            .or_default()
                            .insert(method.to_owned(), function(ret));
                    }
                }
            } else if let Some(rest) = line.strip_prefix("neori.") {
                if let Some(name) = rest.strip_suffix(" = {}") {
                    let mut ty = std::mem::replace(&mut last_type, Type::Unknown);
                    if ty == Type::Unknown {
                        if let Some(class) = class_above.take() {
                            ty = Type::Namespace(class);
                        }
                    }
                    // `neori.util.table = {}`: a namespace inside a namespace.
                    if let Some((namespace, name)) = name.rsplit_once('.') {
                        if let Some(class) = namespace_class(&neori, &members, namespace) {
                            members
                                .entry(class)
                                .or_default()
                                .insert(name.to_owned(), ty);
                        }
                    } else {
                        neori.insert(name.to_owned(), ty);
                    }
                }
            }
            if !line.is_empty() {
                class_above = None;
                last_return = Type::Unknown;
                last_type = Type::Unknown;
                params.clear();
                overloads.clear();
            }
        }
        Api {
            neori,
            members,
            events,
        }
    }

    /// The names that can follow `ty` after `separator` (`.` or `:`).
    fn names_after(&self, ty: &Type, separator: char) -> BTreeSet<&str> {
        match (ty, separator) {
            (Type::Neori, '.') => self.neori.keys().map(String::as_str).collect(),
            (Type::Namespace(class), '.') => self
                .members
                .get(class)
                .into_iter()
                .flatten()
                .map(|(name, _)| name.as_str())
                .collect(),
            (Type::Class(class), _) => self
                .members
                .get(class)
                .into_iter()
                .flatten()
                // Methods follow `:`, fields `.`.
                .filter(|(_, ty)| matches!(ty, Type::Function { .. }) == (separator == ':'))
                .map(|(name, _)| name.as_str())
                .collect(),
            _ => BTreeSet::new(),
        }
    }

    /// The type of `name` after `separator` on a value of type `ty`.
    fn member(&self, ty: &Type, separator: char, name: &str) -> Type {
        match (ty, separator) {
            (Type::Neori, '.') => self.neori.get(name).cloned().unwrap_or(Type::Unknown),
            (Type::Namespace(class), '.') | (Type::Class(class), _) => self
                .members
                .get(class)
                .and_then(|m| m.get(name))
                .cloned()
                .unwrap_or(Type::Unknown),
            _ => Type::Unknown,
        }
    }

    /// The type of argument `index` when calling a value of type `callee`, for a table argument.
    fn argument(&self, callee: &Type, index: usize) -> Type {
        match callee {
            Type::Function { signatures, .. } => signatures
                .iter()
                .filter_map(|signature| signature.get(index))
                .find(|ty| matches!(ty, Type::Class(_) | Type::List(_)))
                .cloned()
                .unwrap_or(Type::Unknown),
            // A config section, called with a table of its own keys: `neori.layout { … }`.
            Type::Class(class) if class.starts_with("config.") && index == 0 => callee.clone(),
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

/// The brackets in the code before the cursor, skipping strings and comments.
#[derive(Default)]
struct Brackets {
    /// Brackets (`(`, `[`, `{`) not closed yet, innermost last, by index.
    open: Vec<usize>,
    /// The bracket each bracket and comma is directly inside of, by index.
    parent: HashMap<usize, usize>,
    /// Whether the code ends inside a string.
    in_string: bool,
}

fn scan_brackets(chars: &[char]) -> Brackets {
    let mut brackets = Brackets::default();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            quote @ ('"' | '\'') => {
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    brackets.in_string = true;
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '(' | '[' | '{' => {
                if let Some(&parent) = brackets.open.last() {
                    brackets.parent.insert(i, parent);
                }
                brackets.open.push(i);
            }
            ',' => {
                if let Some(&parent) = brackets.open.last() {
                    brackets.parent.insert(i, parent);
                }
            }
            ')' | ']' | '}' => {
                brackets.open.pop();
            }
            _ => (),
        }
        i += 1;
    }
    brackets
}

/// The index of the last character before `end` that isn't whitespace.
fn previous_non_space(chars: &[char], end: usize) -> Option<usize> {
    (0..end).rev().find(|&i| !chars[i].is_whitespace())
}

/// The name that ends at `end` (exclusive), and where it starts.
fn name_before(chars: &[char], end: usize) -> (String, usize) {
    let mut start = end;
    while start > 0 && is_name_char(chars[start - 1]) {
        start -= 1;
    }
    (chars[start..end].iter().collect(), start)
}

/// Where the chain of names, `.`, `:` and balanced brackets that ends at `end` starts.
fn chain_start(chars: &[char], end: usize) -> usize {
    let mut start = end;
    loop {
        match start.checked_sub(1).map(|i| chars[i]) {
            Some(c) if is_name_char(c) || c == '.' || c == ':' => start -= 1,
            Some(close @ (')' | ']')) => {
                let open = if close == ')' { '(' } else { '[' };
                let mut depth = 0;
                let mut i = start;
                loop {
                    if i == 0 {
                        return start;
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
            _ => return start,
        }
    }
}

/// Where the chain of names, `.`, `:` and balanced brackets that starts at `start` ends.
fn chain_end(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len() {
        match chars[end] {
            c if is_name_char(c) || c == '.' || c == ':' => end += 1,
            '(' | '[' => match closing(chars, end) {
                Some(close) => end = close + 1,
                None => return end,
            },
            _ => break,
        }
    }
    end
}

/// Works out types in the code before the cursor.
struct Code<'a> {
    api: &'a Api,
    chars: &'a [char],
    brackets: Brackets,
    /// Variables assigned from typed expressions, by name: the last assignment wins.
    vars: HashMap<String, Type>,
}

impl<'a> Code<'a> {
    fn new(api: &'a Api, chars: &'a [char]) -> Self {
        let mut code = Code {
            api,
            chars,
            brackets: scan_brackets(chars),
            vars: HashMap::new(),
        };
        code.infer_vars();
        code
    }

    /// Finds `[local] name = expr`, `for _, name in ipairs(expr)` and `function(name)` given to
    /// an event's `on` or `connect`.
    fn infer_vars(&mut self) {
        let chars = self.chars;
        let mut i = 0;
        while i < chars.len() {
            if !is_name_char(chars[i]) || (i > 0 && is_name_char(chars[i - 1])) {
                i += 1;
                continue;
            }
            let mut end = i;
            while end < chars.len() && is_name_char(chars[end]) {
                end += 1;
            }
            let word: String = chars[i..end].iter().collect();
            match word.as_str() {
                "for" => self.infer_for(end),
                "function" => self.infer_handler(i, end),
                // Not a field (`a.b = …`) nor a method.
                _ if i == 0 || !matches!(chars[i - 1], '.' | ':') => {
                    self.infer_assignment(&word, end)
                }
                _ => (),
            }
            i = end;
        }
    }

    /// `name = expr`, with `end` just after the name.
    fn infer_assignment(&mut self, name: &str, end: usize) {
        let chars = self.chars;
        let skip_spaces = |mut j: usize| {
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            j
        };
        let j = skip_spaces(end);
        if chars.get(j) != Some(&'=') || chars.get(j + 1) == Some(&'=') {
            return;
        }
        let start = skip_spaces(j + 1);
        let expr_end = chain_end(chars, start);
        if expr_end > start {
            let ty = self.chain_type(&chars[start..expr_end]);
            self.vars.insert(name.to_owned(), ty);
        }
    }

    /// `for _, name in ipairs(expr)` or `pairs(expr)`, with `end` just after `for`.
    fn infer_for(&mut self, end: usize) {
        let rest: String = self.chars[end..].iter().collect();
        let Some((names, rest)) = rest.split_once(" in ") else {
            return;
        };
        let names: Vec<&str> = names.split(',').map(str::trim).collect();
        let [_, value] = names[..] else {
            return;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest
            .strip_prefix("ipairs(")
            .or_else(|| rest.strip_prefix("pairs("))
        else {
            return;
        };
        let expr: Vec<char> = rest.chars().collect();
        let expr_end = chain_end(&expr, 0);
        if let Type::List(item) = self.chain_type(&expr[..expr_end]) {
            self.vars.insert(value.to_owned(), *item);
        }
    }

    /// `function(a, b)` after `neori.on("event", ` (or `neori.connect`, or `obj:connect`), with
    /// `start` and `end` around `function`.
    fn infer_handler(&mut self, start: usize, end: usize) {
        let chars = self.chars;
        let rest: String = chars[end..].iter().collect();
        let Some(params) = rest
            .trim_start()
            .strip_prefix('(')
            .and_then(|rest| rest.split_once(')'))
            .map(|(params, _)| params)
        else {
            return;
        };
        let params: Vec<&str> = params.split(',').map(str::trim).collect();

        // Back over `"event", ` to the call's `(`.
        let Some(comma) = previous_non_space(chars, start).filter(|&i| chars[i] == ',') else {
            return;
        };
        let Some(close_quote) =
            previous_non_space(chars, comma).filter(|&i| matches!(chars[i], '"' | '\''))
        else {
            return;
        };
        let Some(open_quote) = (0..close_quote)
            .rev()
            .find(|&i| chars[i] == chars[close_quote])
        else {
            return;
        };
        let event: String = chars[open_quote + 1..close_quote].iter().collect();
        let Some(paren) = previous_non_space(chars, open_quote).filter(|&i| chars[i] == '(') else {
            return;
        };
        let callee: String = chars[..paren].iter().collect();
        let types = if callee.ends_with("neori.on") || callee.ends_with("neori.connect") {
            self.api.events.get(&event).cloned().unwrap_or_default()
        } else if callee.ends_with(":connect") {
            // An object's own signal: its handlers get the object.
            let receiver_end = paren - ":connect".len();
            let receiver = &chars[chain_start(chars, receiver_end)..receiver_end];
            vec![self.chain_type(receiver)]
        } else {
            return;
        };
        for (param, ty) in params.into_iter().zip(types) {
            if !param.is_empty() {
                self.vars.insert(param.to_owned(), ty);
            }
        }
    }

    /// The type of the expression `chain`, like `neori.windows()[1]` or `w:geometry()`.
    fn chain_type(&self, chain: &[char]) -> Type {
        let (name, mut i) = {
            let mut end = 0;
            while end < chain.len() && is_name_char(chain[end]) {
                end += 1;
            }
            (chain[..end].iter().collect::<String>(), end)
        };
        let mut ty = if name == "neori" {
            Type::Neori
        } else {
            self.vars.get(&name).cloned().unwrap_or(Type::Unknown)
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
                    ty = self.api.member(&ty, separator, &name);
                }
                '(' => {
                    let Some(end) = closing(chain, i) else {
                        return Type::Unknown;
                    };
                    ty = match ty {
                        Type::Function { ret, .. } => *ret,
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

    /// The type a table opening at `open` (a `{`) should have, from where it is: an argument of
    /// a call, a field `key = { … }` of another table, or an entry of a list.
    fn table_type(&self, open: usize) -> Type {
        let chars = self.chars;
        let Some(before) = previous_non_space(chars, open) else {
            return Type::Unknown;
        };
        let parent = self.brackets.parent.get(&open).copied();
        match chars[before] {
            // `key = { … }` in a parent table.
            '=' => {
                let Some(key_end) = previous_non_space(chars, before) else {
                    return Type::Unknown;
                };
                let (key, _) = name_before(chars, key_end + 1);
                let Some(parent) = parent.filter(|&p| chars[p] == '{') else {
                    return Type::Unknown;
                };
                match self.api.member(&self.table_type(parent), '.', &key) {
                    Type::List(item) => *item,
                    ty => ty,
                }
            }
            '{' | ',' | '(' => match parent.map(|p| (p, chars[p])) {
                // An entry of a list, or of a table that's one or a list of its type, like
                // `match = { { … }, { … } }`.
                Some((parent, '{')) => match self.table_type(parent) {
                    Type::List(item) => *item,
                    ty => ty,
                },
                // An argument: which one is the commas directly in the call before it.
                Some((parent, '(')) => {
                    let index = (parent + 1..open)
                        .filter(|i| self.brackets.parent.get(i) == Some(&parent))
                        .filter(|&i| chars[i] == ',')
                        .count();
                    let callee = &chars[chain_start(chars, parent)..parent];
                    self.api.argument(&self.chain_type(callee), index)
                }
                _ => Type::Unknown,
            },
            // `f { … }`, a call without parentheses.
            c if is_name_char(c) || c == ')' || c == ']' => {
                let callee = &chars[chain_start(chars, before + 1)..before + 1];
                self.api.argument(&self.chain_type(callee), 0)
            }
            _ => Type::Unknown,
        }
    }
}

/// Completes the Lua code `text`: returns the candidates for the whole text, each one `text`
/// with its last name completed.
///
/// `table_keys` lists the string keys of the global table at a path of names (the globals
/// themselves for an empty path), for what the stubs don't type, like `string.` or the config's
/// own tables.
pub fn complete(text: &str, table_keys: &dyn Fn(&[String]) -> Vec<String>) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();

    // The name being typed, at the end.
    let (partial, partial_start) = name_before(&chars, chars.len());
    let before: String = chars[..partial_start].iter().collect();

    let api = Api::parse();
    let code = Code::new(&api, &chars[..partial_start]);
    if code.brackets.in_string {
        return Vec::new();
    }

    let names: BTreeSet<String> = match chars[..partial_start].last() {
        Some(&separator @ ('.' | ':')) => {
            let chain_end = partial_start - 1;
            let chain = &chars[chain_start(&chars, chain_end)..chain_end];
            let ty = code.chain_type(chain);
            if ty == Type::Unknown {
                global_table_keys(chain, &code, table_keys)
            } else {
                api.names_after(&ty, separator)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            }
        }
        _ => {
            // At a key of a table: right after its `{` or a `,`.
            let table = code.brackets.open.last().copied().filter(|&open| {
                chars[open] == '{'
                    && previous_non_space(&chars, partial_start)
                        .is_some_and(|i| matches!(chars[i], '{' | ',' | ';'))
            });
            match table.map(|open| code.table_type(open)) {
                Some(Type::Class(class)) => api
                    .members
                    .get(&class)
                    .into_iter()
                    .flatten()
                    .filter(|(_, ty)| !matches!(ty, Type::Function { .. }))
                    .map(|(name, _)| name.clone())
                    .collect(),
                _ => {
                    let mut names: BTreeSet<String> = table_keys(&[]).into_iter().collect();
                    names.insert("neori".to_owned());
                    names.extend(code.vars.keys().cloned());
                    names
                }
            }
        }
    };

    names
        .into_iter()
        .filter(|name| name.starts_with(&partial))
        .map(|name| format!("{before}{name}"))
        .collect()
}

/// A function of the stubs: its parameters as `(key, action, options?)`, and its documentation.
struct FunctionDoc {
    params: String,
    returns: Option<String>,
    doc: String,
}

/// The documented functions of the stubs, by name: `neori.bind`, `neori.rules.add`, and methods
/// as `Window:title`.
fn function_docs() -> HashMap<String, FunctionDoc> {
    let mut docs = HashMap::new();
    let mut block: Vec<&str> = Vec::new();
    for line in STUBS.lines() {
        if let Some(text) = line.strip_prefix("---") {
            block.push(text);
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("function ")
            .and_then(|f| f.strip_suffix(" end"))
        {
            let (name, args) = rest.split_once('(').unwrap_or((rest, ""));
            let args = args.trim_end_matches(')');
            // Optional parameters are marked in their `---@param name? …`.
            let optional = |arg: &str| {
                block
                    .iter()
                    .any(|l| l.starts_with(&format!("@param {arg}? ")))
            };
            let params: Vec<String> = args
                .split(", ")
                .filter(|a| !a.is_empty())
                .map(|a| {
                    if optional(a) {
                        format!("{a}?")
                    } else {
                        a.to_owned()
                    }
                })
                .collect();
            let returns: Vec<&str> = block
                .iter()
                .filter_map(|l| l.strip_prefix("@return "))
                .map(|r| r.split_whitespace().next().unwrap_or(""))
                .collect();
            let doc = block
                .iter()
                .filter(|l| !l.starts_with('@') && !l.starts_with('|'))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            docs.insert(
                name.to_owned(),
                FunctionDoc {
                    params: format!("({})", params.join(", ")),
                    returns: (!returns.is_empty()).then(|| returns.join(", ")),
                    doc: doc.trim().to_owned(),
                },
            );
        }
        block.clear();
    }
    docs
}

/// The name of the function called by the `(` that ends `chars`, as [`function_docs`] has it.
fn callee_name(code: &Code, chars: &[char]) -> Option<String> {
    let paren = chars.len().checked_sub(1)?;
    if chars[paren] != '(' {
        return None;
    }
    let (name, name_start) = name_before(chars, paren);
    if name.is_empty() {
        return None;
    }
    let separator = name_start.checked_sub(1).map(|i| chars[i]);
    match separator {
        Some(separator @ ('.' | ':')) => {
            let receiver_end = name_start - 1;
            let receiver = &chars[chain_start(chars, receiver_end)..receiver_end];
            match code.chain_type(receiver) {
                // `w:title(`: a method of the receiver's class.
                Type::Class(class) if separator == ':' => Some(format!("{class}:{name}")),
                // `neori.bind(`, `neori.rules.add(`: written out in full.
                Type::Neori | Type::Namespace(_) => {
                    let receiver: String = receiver.iter().collect();
                    Some(format!("{receiver}.{name}"))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The parameters still to type when `text` ends right after the `(` of a call to a function of
/// the API, as a hint: `"key, action, options?)"` for `neori.bind(`.
pub fn signature_hint(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let api = Api::parse();
    let code = Code::new(&api, &chars);
    if code.brackets.in_string {
        return None;
    }
    let name = callee_name(&code, &chars)?;
    let docs = function_docs();
    let doc = docs.get(&name)?;
    let params = doc.params.strip_prefix('(').unwrap_or(&doc.params);
    Some(match &doc.returns {
        Some(returns) => format!("{params} -> {returns}"),
        None => params.to_owned(),
    })
}

/// The documentation of a function of the API (`neori.bind`, `Window:title`): its signature,
/// then its description.
pub fn doc(name: &str) -> Option<String> {
    let docs = function_docs();
    let doc = docs.get(name)?;
    let mut text = format!("{name}{}", doc.params);
    if let Some(returns) = &doc.returns {
        text.push_str(&format!(" -> {returns}"));
    }
    if !doc.doc.is_empty() {
        text.push_str("\n\n");
        text.push_str(&doc.doc);
    }
    Some(text)
}

/// The keys of the global table `chain` names, when it's a plain path like `string` or
/// `my.helpers` that isn't `neori` nor a variable of the code.
fn global_table_keys(
    chain: &[char],
    code: &Code,
    table_keys: &dyn Fn(&[String]) -> Vec<String>,
) -> BTreeSet<String> {
    let chain: String = chain.iter().collect();
    let path: Vec<String> = chain.split('.').map(str::to_owned).collect();
    let plain = path
        .iter()
        .all(|name| !name.is_empty() && name.chars().all(is_name_char));
    if !plain || path[0] == "neori" || code.vars.contains_key(&path[0]) {
        return BTreeSet::new();
    }
    table_keys(&path).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_keys(path: &[String]) -> Vec<String> {
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        let keys: &[&str] = match path[..] {
            [] => &["my_helper", "neori", "string"],
            ["string"] => &["format", "find", "rep"],
            _ => &[],
        };
        keys.iter().map(|&key| key.to_owned()).collect()
    }

    fn complete_names(text: &str) -> Vec<String> {
        complete(text, &table_keys)
    }

    fn offers(text: &str, candidate: &str) -> bool {
        complete_names(text).iter().any(|c| c == candidate)
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
        assert!(offers("neori.", "neori.misc"));
        assert!(offers("neori.", "neori.layout"));
    }

    #[test]
    fn completes_methods_from_return_types() {
        assert!(offers(
            "neori.focused_window():ti",
            "neori.focused_window():title"
        ));
        // A list, indexed.
        assert!(offers(
            "neori.windows()[1]:app",
            "neori.windows()[1]:app_id"
        ));
        // A method returning another class.
        assert!(offers(
            "neori.focused_window():geometry().w",
            "neori.focused_window():geometry().width"
        ));
        assert_eq!(
            complete_names("neori.focused_output():work"),
            ["neori.focused_output():workarea"]
        );
        // Arguments with brackets of their own.
        assert!(offers(
            "neori.window_at(f(1), 2):ti",
            "neori.window_at(f(1), 2):title"
        ));
    }

    #[test]
    fn completes_config_section_keys() {
        // From neori-config.lua, generated from the config structs.
        assert!(offers("neori.layout.ga", "neori.layout.gaps"));
        assert!(offers(
            "neori.layout.focus_ring.active_",
            "neori.layout.focus_ring.active_gradient"
        ));
        assert!(offers(
            "neori.input.touchpad.accel_",
            "neori.input.touchpad.accel_profile"
        ));
    }

    #[test]
    fn methods_after_colon_fields_after_dot() {
        assert!(offers(
            "neori.focused_window():",
            "neori.focused_window():close"
        ));
        assert!(!offers(
            "neori.focused_window().",
            "neori.focused_window().close"
        ));
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
        // Nothing inside a string.
        assert!(complete_names("print('neori.fo").is_empty());
    }

    #[test]
    fn completes_variables_from_what_they_were_assigned() {
        assert_eq!(
            complete_names("local w = neori.focused_window(); w:ti"),
            ["local w = neori.focused_window(); w:title"]
        );
        assert!(offers(
            "out = neori.focused_output()\nprint(out:na",
            "out = neori.focused_output()\nprint(out:name"
        ));
        // The variable's own name completes too.
        assert!(offers(
            "local window = neori.focused_window(); win",
            "local window = neori.focused_window(); window"
        ));
        // `==` isn't an assignment.
        assert!(complete_names("if w == neori.focused_window() then w:").is_empty());
    }

    #[test]
    fn completes_loop_variables_and_handler_parameters() {
        assert!(offers(
            "for _, w in ipairs(neori.windows()) do print(w:ti",
            "for _, w in ipairs(neori.windows()) do print(w:title"
        ));
        assert!(offers(
            "neori.on(\"window-opened\", function(w) w:ti",
            "neori.on(\"window-opened\", function(w) w:title"
        ));
        assert!(offers(
            "neori.on(\"layer-added\", function(ls) ls:nam",
            "neori.on(\"layer-added\", function(ls) ls:namespace"
        ));
        // An object's own signal gets the object.
        assert!(offers(
            "local o = neori.focused_output(); o:connect(\"mode-changed\", function(x) x:na",
            "local o = neori.focused_output(); o:connect(\"mode-changed\", function(x) x:name"
        ));
    }

    #[test]
    fn completes_namespace_functions_and_their_tables() {
        assert_eq!(
            complete_names("neori.rules.a"),
            ["neori.rules.add", "neori.rules.apply"]
        );
        assert!(!offers("neori.ru", "neori.rules.add"));
        assert!(offers("neori.util.", "neori.util.table"));
        assert!(offers("neori.util.", "neori.util.dump"));
        assert_eq!(
            complete_names("neori.util.table.cl"),
            ["neori.util.table.clone"]
        );
        assert!(offers(
            "local t = neori.util.geometry; t.in_d",
            "local t = neori.util.geometry; t.in_direction"
        ));
        assert!(offers(
            "neori.rules.add { wh",
            "neori.rules.add { where_not"
        ));
        assert!(offers(
            "neori.rules.add { where = { app",
            "neori.rules.add { where = { app_id"
        ));
        assert!(offers(
            "neori.rules.add { set = { flo",
            "neori.rules.add { set = { floating"
        ));
    }

    #[test]
    fn hints_the_parameters_of_calls() {
        assert_eq!(
            signature_hint("neori.bind(").as_deref(),
            Some("key, action, options?)")
        );
        assert_eq!(
            signature_hint("local w = neori.focused_window(); w:set_opacity(").as_deref(),
            Some("opacity)")
        );
        assert_eq!(
            signature_hint("neori.rules.add(").as_deref(),
            Some("rule) -> string")
        );
        assert_eq!(
            signature_hint("neori.focused_window(").as_deref(),
            Some(") -> neori.Window?")
        );
        // Only right after the `(`, and not inside strings or for unknown functions.
        assert_eq!(signature_hint("neori.bind(\"Mod+Q\", "), None);
        assert_eq!(signature_hint("print(\"neori.bind("), None);
        assert_eq!(signature_hint("string.format("), None);
    }

    #[test]
    fn documents_functions_and_methods() {
        let bind = doc("neori.bind").unwrap();
        assert!(
            bind.starts_with("neori.bind(key, action, options?)\n\nBinds a key"),
            "{bind}"
        );
        let title = doc("Window:title").unwrap();
        assert!(title.starts_with("Window:title() -> string?"), "{title}");
        assert_eq!(doc("neori.nothing"), None);
    }

    #[test]
    fn completes_config_table_keys() {
        assert_eq!(complete_names("neori.layout { ga"), ["neori.layout { gaps"]);
        assert_eq!(complete_names("neori.layout({ ga"), ["neori.layout({ gaps"]);
        assert!(offers(
            "neori.layout { gaps = 8, focus_ring = { act",
            "neori.layout { gaps = 8, focus_ring = { active_gradient"
        ));
        assert!(offers(
            "neori.window_rule { match = { app",
            "neori.window_rule { match = { app_id"
        ));
        assert!(offers(
            "neori.window_rule { match = { { app",
            "neori.window_rule { match = { { app_id"
        ));
        assert!(offers(
            "neori.output(\"DP-1\", { sca",
            "neori.output(\"DP-1\", { scale"
        ));
        assert!(offers(
            "neori.window_rules({ { open_f",
            "neori.window_rules({ { open_floating"
        ));
        // A value isn't a key.
        assert!(!offers(
            "neori.layout { gaps = ga",
            "neori.layout { gaps = gaps"
        ));
    }

    #[test]
    fn completes_keys_of_other_global_tables() {
        assert_eq!(complete_names("string.f"), ["string.find", "string.format"]);
        assert!(offers("print(string.re", "print(string.rep"));
        assert!(complete_names("nothing.").is_empty());
    }
}
