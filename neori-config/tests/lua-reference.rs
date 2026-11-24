//! Generates the wiki's Lua API reference, `docs/wiki/Lua-API-Reference.md`, from the LuaLS
//! definitions in `resources/lua/neori.lua`, and fails when the page is out of date.
//!
//! To update the page after changing the definitions:
//!
//!     NEORI_UPDATE_LUA_REFERENCE=1 cargo test -p neori-config --test lua-reference
//!
//! The definitions are made of:
//!
//! - banners (`----` lines around `-- Title`), the page's sections;
//! - `-- Title` comments, its subsections, with any `--` lines after them as text;
//! - `---` doc blocks with annotations (`@param`, `@return`, `@field`, …), each before the
//!   declaration it describes (`function neori.x(…) end`, `neori.x = {}`, `local Class = {}`), or
//!   on their own for classes and aliases.

use std::fmt::Write as _;
use std::path::PathBuf;

const STUBS: &str = include_str!("../../resources/lua/neori.lua");

/// A name, its type and what it is: a parameter, a return value or a field.
struct Typed {
    name: String,
    ty: String,
    doc: String,
}

#[derive(Default)]
struct DocBlock {
    /// The text before the annotations.
    doc: Vec<String>,
    class: Option<String>,
    alias: Option<String>,
    /// The values of an alias: the value and its description.
    alias_values: Vec<(String, String)>,
    ty: Option<String>,
    params: Vec<Typed>,
    returns: Vec<Typed>,
    fields: Vec<Typed>,
    overloads: Vec<String>,
}

/// Splits `text` after a type: `fun(a: T): R rest` gives `("fun(a: T): R", "rest")`. A type ends
/// at whitespace outside brackets, unless the whitespace follows `:`, `,` or `|`, or precedes `|`.
fn split_type(text: &str) -> (&str, &str) {
    let mut depth = 0i32;
    let mut in_string = false;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(at, c)) in chars.iter().enumerate() {
        match c {
            '"' => in_string = !in_string,
            _ if in_string => {}
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                let before = text[..at].trim_end().chars().last();
                let after = chars[i + 1..]
                    .iter()
                    .map(|&(_, c)| c)
                    .find(|c| !c.is_whitespace());
                if matches!(before, Some(':' | ',' | '|')) || after == Some('|') {
                    continue;
                }
                return (&text[..at], text[at..].trim());
            }
            _ => {}
        }
    }
    (text, "")
}

/// Parses `name type description`.
fn typed(text: &str) -> Typed {
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    let (ty, doc) = split_type(rest.trim());
    Typed {
        name: name.to_owned(),
        ty: ty.to_owned(),
        doc: doc.to_owned(),
    }
}

impl DocBlock {
    fn parse(lines: &[&str]) -> Self {
        let mut block = DocBlock::default();
        // Text after `@class`, for the next field.
        let mut pending = Vec::new();
        for line in lines {
            let line = line.strip_prefix("---").unwrap();
            if let Some(class) = line.strip_prefix("@class ") {
                block.class = Some(class.split([' ', ':']).next().unwrap().to_owned());
            } else if let Some(alias) = line.strip_prefix("@alias ") {
                let (name, ty) = alias.split_once(' ').unwrap_or((alias, ""));
                block.alias = Some(name.to_owned());
                if !ty.is_empty() {
                    block.ty = Some(ty.trim().to_owned());
                }
            } else if let Some(value) = line.strip_prefix("| ") {
                let (value, doc) = value.split_once(" # ").unwrap_or((value, ""));
                block.alias_values.push((value.to_owned(), doc.to_owned()));
            } else if let Some(ty) = line.strip_prefix("@type ") {
                block.ty = Some(ty.to_owned());
            } else if let Some(param) = line.strip_prefix("@param ") {
                block.params.push(typed(param));
            } else if let Some(ret) = line.strip_prefix("@return ") {
                let (ty, rest) = split_type(ret);
                block.returns.push(Typed {
                    name: String::new(),
                    ty: ty.to_owned(),
                    doc: rest.to_owned(),
                });
            } else if let Some(field) = line.strip_prefix("@field ") {
                let mut field = typed(field);
                if !pending.is_empty() {
                    let doc = pending.join(" ");
                    field.doc = if field.doc.is_empty() {
                        doc
                    } else {
                        format!("{doc} {}", field.doc)
                    };
                    pending.clear();
                }
                block.fields.push(field);
            } else if let Some(overload) = line.strip_prefix("@overload ") {
                block.overloads.push(overload.to_owned());
            } else if line.starts_with('@') {
                // `@generic`, `@meta`: nothing to show.
            } else if block.class.is_some() {
                pending.push(line.to_owned());
            } else {
                block.doc.push(line.to_owned());
            }
        }
        block
    }
}

/// Escapes a type or text for a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn code(text: &str) -> String {
    // A backtick inside would end the span; the definitions don't use them in types.
    format!("`{text}`")
}

/// The text of a doc block, as Markdown.
fn doc_text(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text.trim_start_matches('\n').to_owned()
}

fn write_typed_list(out: &mut String, title: &str, items: &[Typed]) {
    if items.is_empty() {
        return;
    }
    writeln!(out, "{title}:\n").unwrap();
    for item in items {
        let mut line = String::from("- ");
        if !item.name.is_empty() {
            write!(line, "{} ", code(&item.name)).unwrap();
        }
        write!(line, "{}", code(&item.ty)).unwrap();
        if !item.doc.is_empty() {
            write!(line, ": {}", item.doc).unwrap();
        }
        writeln!(out, "{line}").unwrap();
    }
    out.push('\n');
}

fn write_fields(out: &mut String, fields: &[Typed]) {
    if fields.is_empty() {
        return;
    }
    out.push_str("| field | type | |\n|---|---|---|\n");
    for field in fields {
        writeln!(
            out,
            "| {} | {} | {} |",
            code(&field.name),
            code(&cell(&field.ty)),
            cell(&field.doc)
        )
        .unwrap();
    }
    out.push('\n');
}

fn write_overloads(out: &mut String, overloads: &[String]) {
    if overloads.is_empty() {
        return;
    }
    out.push_str("Can also be called as:\n\n```\n");
    for overload in overloads {
        writeln!(out, "{overload}").unwrap();
    }
    out.push_str("```\n\n");
}

/// `function neori.bind(key, action, options) end` with its block, as a heading and text.
fn write_function(out: &mut String, declaration: &str, block: &DocBlock) {
    let (name, args) = declaration.split_once('(').unwrap();
    let args = args.split(')').next().unwrap();
    let args: Vec<String> = args
        .split(", ")
        .filter(|a| !a.is_empty())
        .map(|arg| {
            let optional = block.params.iter().any(|p| p.name == format!("{arg}?"));
            if optional {
                format!("{arg}?")
            } else {
                arg.to_owned()
            }
        })
        .collect();
    writeln!(out, "#### `{name}({})`\n", args.join(", ")).unwrap();
    let doc = doc_text(&block.doc);
    if !doc.is_empty() {
        writeln!(out, "{doc}\n").unwrap();
    }
    write_typed_list(out, "Parameters", &block.params);
    match &block.returns[..] {
        // A single return value fits on a line.
        [ret] => {
            let doc = if ret.doc.is_empty() {
                String::new()
            } else {
                format!(": {}", ret.doc)
            };
            writeln!(out, "Returns {}{doc}.\n", code(&ret.ty)).unwrap();
        }
        returns => write_typed_list(out, "Returns", returns),
    }
    write_overloads(out, &block.overloads);
}

/// A class, alias or namespace, as a heading and text.
fn write_type(out: &mut String, heading: &str, block: &DocBlock) {
    writeln!(out, "#### `{heading}`\n").unwrap();
    let doc = doc_text(&block.doc);
    if !doc.is_empty() {
        writeln!(out, "{doc}\n").unwrap();
    }
    if let Some(ty) = &block.ty {
        writeln!(out, "Type: {}\n", code(ty)).unwrap();
    }
    if !block.alias_values.is_empty() {
        out.push_str("| value | |\n|---|---|\n");
        for (value, doc) in &block.alias_values {
            writeln!(out, "| {} | {} |", code(&cell(value)), cell(doc)).unwrap();
        }
        out.push('\n');
    }
    write_fields(out, &block.fields);
    write_overloads(out, &block.overloads);
}

/// A GitHub wiki heading anchor.
fn anchor(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn generate() -> String {
    let lines: Vec<&str> = STUBS.lines().collect();
    let mut body = String::new();
    let mut sections = Vec::new();
    // The local variable each class is declared on, for `function Window:title()`.
    let mut i = 0;

    // Everything before the first banner is the file's own introduction.
    while i < lines.len() && !lines[i].starts_with("----") {
        i += 1;
    }

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("----") {
            // A banner: `----`, `-- Title`, `----`.
            let title = lines[i + 1].trim_start_matches("--").trim();
            writeln!(body, "## {title}\n").unwrap();
            sections.push(title.to_owned());
            i += 3;
        } else if line.starts_with("---") {
            let start = i;
            while i < lines.len() && lines[i].starts_with("---") && !lines[i].starts_with("----") {
                i += 1;
            }
            let block = DocBlock::parse(&lines[start..i]);
            let declaration = lines.get(i).copied().unwrap_or("");
            if let Some(function) = declaration
                .strip_prefix("function ")
                .and_then(|f| f.strip_suffix(" end"))
            {
                write_function(&mut body, function, &block);
                i += 1;
            } else if let Some(name) = declaration.strip_suffix(" = {}") {
                i += 1;
                if name == "neori" {
                    continue;
                }
                let name = name.strip_prefix("local ").map_or_else(
                    // `neori.layout = {}`: a section or namespace.
                    || name.to_owned(),
                    // `local Window = {}`: a class.
                    |_| block.class.clone().unwrap_or_default(),
                );
                write_type(&mut body, &name, &block);
            } else if let Some(alias) = &block.alias {
                write_type(&mut body, alias, &block);
            } else if let Some(class) = &block.class {
                write_type(&mut body, class, &block);
            } else {
                panic!(
                    "a doc block without a declaration at line {}: {:?}",
                    start + 1,
                    lines[start]
                );
            }
        } else if let Some(text) = line.strip_prefix("--") {
            // `-- Title`, then any text up to the next line that isn't a comment.
            writeln!(body, "### {}\n", text.trim()).unwrap();
            i += 1;
            let mut prose = Vec::new();
            while i < lines.len() && lines[i].starts_with("--") && !lines[i].starts_with("---") {
                prose.push(lines[i].strip_prefix("--").unwrap());
                i += 1;
            }
            write_prose(&mut body, &prose);
        } else if let Some(function) = line
            .strip_prefix("function ")
            .and_then(|f| f.strip_suffix(" end"))
        {
            // A function without a doc block.
            write_function(&mut body, function, &DocBlock::default());
            i += 1;
        } else {
            i += 1;
        }
    }

    let mut out = String::new();
    out.push_str(
        "<!-- Generated from resources/lua/neori.lua by neori-config/tests/lua-reference.rs; \
         don't edit. Update it with:\n     \
         NEORI_UPDATE_LUA_REFERENCE=1 cargo test -p neori-config --test lua-reference -->\n\n",
    );
    out.push_str(
        "Every function, method, type and signal of neori's Lua API, generated from the type \
         definitions that editors use (see [Editor support](./Lua-Scripting.md#editor-support)). \
         For how to use them, with examples, see [Lua Scripting](./Lua-Scripting.md). The keys \
         of the config sections are described on the Configuration pages.\n\n",
    );
    for section in &sections {
        writeln!(out, "- [{section}](#{})", anchor(section)).unwrap();
    }
    out.push('\n');
    out.push_str(&body);
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// The `--` lines after a subsection title: text, with lines indented by four spaces as code.
fn write_prose(out: &mut String, lines: &[&str]) {
    // The text is often set apart from the title by an empty `--` line.
    let lines = match lines.iter().position(|l| !l.trim().is_empty()) {
        Some(first) => &lines[first..],
        None => &[],
    };
    let mut in_code = false;
    for line in lines {
        let is_code = line.starts_with("     ");
        if is_code != in_code {
            out.push_str(if is_code { "```lua\n" } else { "```\n" });
            in_code = is_code;
        }
        let text = if is_code { &line[5..] } else { line.trim() };
        writeln!(out, "{text}").unwrap();
    }
    if in_code {
        out.push_str("```\n");
    }
    if !lines.is_empty() {
        out.push('\n');
    }
}

#[test]
fn lua_api_reference_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/wiki/Lua-API-Reference.md");
    let generated = generate();
    if std::env::var_os("NEORI_UPDATE_LUA_REFERENCE").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == generated,
        "docs/wiki/Lua-API-Reference.md is out of date; update it with \
         NEORI_UPDATE_LUA_REFERENCE=1 cargo test -p neori-config --test lua-reference"
    );
}

#[test]
fn types_split_where_they_end() {
    assert_eq!(split_type("string A key"), ("string", "A key"));
    assert_eq!(
        split_type("fun(rule: neori.Rule): string The id"),
        ("fun(rule: neori.Rule): string", "The id")
    );
    assert_eq!(
        split_type("{ name?: string, width?: integer } Options"),
        ("{ name?: string, width?: integer }", "Options")
    );
    assert_eq!(
        split_type("\"a b\"|string desc"),
        ("\"a b\"|string", "desc")
    );
    assert_eq!(split_type("integer|string"), ("integer|string", ""));
}
