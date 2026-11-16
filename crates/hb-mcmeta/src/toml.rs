//! A small TOML reader for mod metadata.
//!
//! Not a general TOML implementation. It covers what `mods.toml` and
//! `neoforge.mods.toml` files contain in practice:
//!
//! * tables (`[mods]`, `[[mods]]`, `[dependencies.example]`, quoted keys with
//!   dots such as `[dependencies."my.mod"]`),
//! * strings (basic, literal, multi-line, with `\"` and `\\` escapes),
//! * integers, floats, booleans,
//! * arrays, including multi-line arrays of strings and of tables (Forge's
//!   `[[dependencies.x]]` inside an array-of-tables context).
//!
//! It deliberately accepts two things a strict TOML parser refuses, because
//! real jars contain them and refusing would block a legitimate port:
//!
//! * `${file.jarVersion}` and similar placeholders, which Forge substitutes at
//!   load time — kept as literal strings and reported to the caller;
//! * a file that ends mid-table (a truncated download), reported as an error
//!   with the line number rather than a panic.

use hb_json::{Object, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct TomlError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TomlError {}

pub fn parse(text: &str) -> Result<Value, TomlError> {
    let mut parser = Parser {
        // One-based line numbers, as every editor and compiler shows them.
        lines: text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect(),
        at: 0,
        root: Value::Obj(Object::new()),
        current_path: Vec::new(),
    };
    parser.run()?;
    Ok(parser.root)
}

struct Parser<'a> {
    lines: Vec<(usize, &'a str)>,
    at: usize,
    root: Value,
    /// Path of the table the current `key = value` lines belong to.
    current_path: Vec<String>,
}

impl<'a> Parser<'a> {
    fn run(&mut self) -> Result<(), TomlError> {
        let mut last_array_table: Option<Vec<String>> = None;
        while self.at < self.lines.len() {
            let (line_number, raw) = self.lines[self.at];
            self.at += 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("[[") {
                let Some(name) = rest.strip_suffix("]]") else {
                    return Err(TomlError {
                        line: line_number,
                        message: "array-of-tables header is not closed with `]]`".into(),
                    });
                };
                let path = self.parse_header_path(name, line_number)?;
                // Append a new element to the array at `path`.
                let array_path = path[..path.len() - 1].to_vec();
                let last = path[path.len() - 1].clone();
                let array = self.navigate(&array_path, true, line_number)?;
                if let Some(object) = array.as_obj_mut() {
                    let entry = object
                        .get_mut(&last)
                        .map(|v| match v {
                            Value::Arr(items) => items,
                            _ => unreachable!(),
                        });
                    match entry {
                        Some(items) => items.push(Value::Obj(Object::new())),
                        None => {
                            object.insert(last.clone(), Value::Arr(vec![Value::Obj(Object::new())]));
                        }
                    }
                }
                let mut element_path = array_path.clone();
                element_path.push(last.clone());
                // Track the index so `key = value` lands in the new element.
                let index = self
                    .root_obj_path(&array_path)
                    .and_then(|o| o.get(&last))
                    .and_then(|v| match v {
                        Value::Arr(items) => Some(items.len().saturating_sub(1)),
                        _ => None,
                    })
                    .unwrap_or(0);
                self.current_path = element_path;
                self.current_path.push(index.to_string());
                last_array_table = Some(path);
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let Some(name) = rest.strip_suffix(']') else {
                    return Err(TomlError {
                        line: line_number,
                        message: "table header is not closed with `]`".into(),
                    });
                };
                let path = self.parse_header_path(name, line_number)?;
                // Create the table so later dotted keys resolve.
                self.navigate(&path, true, line_number)?;
                self.current_path = path;
                last_array_table = None;
                continue;
            }
            // `key = value`, possibly continuing onto following lines when the
            // value is an unclosed array.
            let Some((key, rest)) = split_key(line) else {
                return Err(TomlError {
                    line: line_number,
                    message: format!("expected `key = value`, found `{line}`"),
                });
            };
            let mut value_text = rest.trim().to_owned();
            // A value may continue onto following lines when it is an unclosed
            // array, or an unclosed multi-line string, which is how Forge
            // metadata usually writes `description`.
            let mut open = count_unclosed(&value_text);
            while open > 0 || has_unclosed_multiline(&value_text) {
                if self.at >= self.lines.len() {
                    return Err(TomlError {
                        line: line_number,
                        message: "array value is not closed before the end of the file".into(),
                    });
                }
                let (_, more) = self.lines[self.at];
                self.at += 1;
                value_text.push('\n');
                value_text.push_str(more);
                open = count_unclosed(&value_text);
                if value_text.len() > 1 << 20 {
                    return Err(TomlError {
                        line: line_number,
                        message: "value is longer than 1 MiB".into(),
                    });
                }
            }
            let value = parse_value(&value_text).ok_or_else(|| TomlError {
                line: line_number,
                message: format!("cannot read the value `{value_text}`"),
            })?;
            self.assign(&key, value, line_number, last_array_table.is_some())?;
        }
        Ok(())
    }

    fn parse_header_path(&self, name: &str, line: usize) -> Result<Vec<String>, TomlError> {
        let parts = split_dotted(name).ok_or_else(|| TomlError {
            line,
            message: format!("cannot read the table name `{name}`"),
        })?;
        if parts.is_empty() {
            return Err(TomlError {
                line,
                message: "empty table name".into(),
            });
        }
        Ok(parts)
    }

    fn root_obj_path(&mut self, path: &[String]) -> Option<&mut Object> {
        let mut node = &mut self.root;
        for segment in path {
            node = node.as_obj_mut()?.get_mut(segment)?;
        }
        node.as_obj_mut()
    }

    fn navigate(
        &mut self,
        path: &[String],
        create: bool,
        line: usize,
    ) -> Result<&mut Value, TomlError> {
        let mut node = &mut self.root;
        for segment in path {
            if !node.is_obj() {
                return Err(TomlError {
                    line,
                    message: format!("`{segment}` is inside something that is not a table"),
                });
            }
            let object = node.as_obj_mut().expect("checked");
            if !object.contains_key(segment) {
                if !create {
                    return Err(TomlError {
                        line,
                        message: format!("no table `{segment}`"),
                    });
                }
                object.insert(segment.clone(), Value::Obj(Object::new()));
            }
            node = object.get_mut(segment).expect("just inserted");
        }
        Ok(node)
    }

    fn assign(
        &mut self,
        key: &str,
        value: Value,
        line: usize,
        in_array_table: bool,
    ) -> Result<(), TomlError> {
        let parts = split_dotted(key).ok_or_else(|| TomlError {
            line,
            message: format!("cannot read the key `{key}`"),
        })?;
        let mut path = self.current_path.clone();
        // Inside an array-of-tables element, the last segment is an index
        // marker that `navigate` must follow as an array index.
        if let Some(last) = path.last() {
            if last.chars().all(|c| c.is_ascii_digit()) && in_array_table {
                let index: usize = last.parse().unwrap_or(0);
                let array_path = path[..path.len() - 1].to_vec();
                let array = self.navigate(&array_path, false, line)?;
                let object = match array {
                    Value::Arr(items) => items.get_mut(index),
                    _ => None,
                };
                let Some(object) = object.and_then(|v| v.as_obj_mut()) else {
                    return Err(TomlError {
                        line,
                        message: "array-of-tables element vanished".into(),
                    });
                };
                return assign_within(object, &parts, value, line);
            }
        }
        // Plain table: resolve `current_path`, then the dotted key.
        path.truncate(self.current_path.len());
        let table = self.navigate(&self.current_path.clone(), true, line)?;
        let Some(object) = table.as_obj_mut() else {
            return Err(TomlError {
                line,
                message: "current table is not a table".into(),
            });
        };
        assign_within(object, &parts, value, line)
    }
}

fn assign_within(
    object: &mut Object,
    parts: &[String],
    value: Value,
    line: usize,
) -> Result<(), TomlError> {
    let (last, parents) = parts.split_last().expect("non-empty");
    let mut node = object;
    for segment in parents {
        if !node.contains_key(segment) {
            node.insert(segment.clone(), Value::Obj(Object::new()));
        }
        let Some(next) = node.get_mut(segment) else {
            return Err(TomlError {
                line,
                message: format!("`{segment}` is not a table"),
            });
        };
        let Some(next) = next.as_obj_mut() else {
            return Err(TomlError {
                line,
                message: format!("`{segment}` is not a table"),
            });
        };
        node = next;
    }
    node.insert(last.clone(), value);
    Ok(())
}

/// Removes a `#` comment that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'\\' if in_basic => escaped = true,
            b'"' if !in_literal => in_basic = !in_basic,
            b'\'' if !in_basic => in_literal = !in_literal,
            b'#' if !in_basic && !in_literal => return &line[..i],
            _ => {}
        }
    }
    line
}

fn split_key(line: &str) -> Option<(String, &str)> {
    let bytes = line.as_bytes();
    let mut in_basic = false;
    let mut in_literal = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' if !in_literal => in_basic = !in_basic,
            b'\'' if !in_basic => in_literal = !in_literal,
            b'=' if !in_basic && !in_literal => {
                let key = line[..i].trim().trim_matches('"').to_owned();
                return Some((key, &line[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

/// Splits `a.b."c.d"` into `[a, b, c.d]`.
fn split_dotted(text: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_basic = false;
    let mut in_literal = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_literal => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '.' if !in_basic && !in_literal => {
                parts.push(current.trim().to_owned());
                current.clear();
            }
            '\\' if in_basic => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            _ => current.push(c),
        }
    }
    if in_basic || in_literal {
        return None;
    }
    parts.push(current.trim().to_owned());
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(parts)
}

/// Whether a triple-quoted string is opened but not closed.
fn has_unclosed_multiline(text: &str) -> bool {
    ["\"\"\"", "'''"]
        .iter()
        .any(|quote| text.matches(quote).count() % 2 == 1)
}

/// Number of array brackets still open, ignoring strings.
fn count_unclosed(text: &str) -> i32 {
    let mut depth = 0;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_basic => escaped = true,
            '"' if !in_literal => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '[' if !in_basic && !in_literal => depth += 1,
            ']' if !in_basic && !in_literal => depth -= 1,
            _ => {}
        }
    }
    depth.max(0)
}

fn parse_value(text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with("\"\"\"") || text.starts_with("'''") {
        let quote = &text[..3];
        let inner = text.strip_prefix(quote)?.strip_suffix(quote)?;
        // TOML trims one newline right after the opening delimiter.
        let inner = inner.strip_prefix('\n').unwrap_or(inner);
        let inner = if quote == "\"\"\"" {
            unescape_basic(inner)
        } else {
            inner.to_owned()
        };
        return Some(Value::Str(inner));
    }
    if let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        return Some(Value::Str(unescape_basic(inner)));
    }
    if let Some(inner) = text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return Some(Value::Str(inner.to_owned()));
    }
    if text.starts_with('[') {
        return parse_array(text);
    }
    match text {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        _ => {}
    }
    // TOML allows underscores in numbers; JSON does not.
    let cleaned = text.replace('_', "");
    if hb_json::number::Number::parse(&cleaned).is_ok() {
        return Some(Value::Num(hb_json::number::Number::parse_unchecked(&cleaned)));
    }
    // A bare date or anything else TOML allows but metadata never uses: keep
    // it as text rather than failing the whole file.
    Some(Value::Str(text.to_owned()))
}

fn parse_array(text: &str) -> Option<Value> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_basic => {
                current.push(c);
                escaped = true;
            }
            '"' if !in_literal => {
                in_basic = !in_basic;
                current.push(c);
            }
            '\'' if !in_basic => {
                in_literal = !in_literal;
                current.push(c);
            }
            '[' if !in_basic && !in_literal => {
                depth += 1;
                current.push(c);
            }
            ']' if !in_basic && !in_literal => {
                depth -= 1;
                current.push(c);
            }
            ',' if !in_basic && !in_literal && depth == 0 => {
                if !current.trim().is_empty() {
                    items.push(parse_value(&current)?);
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        items.push(parse_value(&current)?);
    }
    Some(Value::Arr(items))
}

fn unescape_basic(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => out.push_str(&hex),
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_neoforge_manifest() {
        let text = r#"
modLoader = "javafml"
loaderVersion = "[1,)"

[[mods]]
modId = "example"
version = "1.2.3"
displayName = "Example Mod"
authors = "Someone, Someone Else"
description = '''A mod with a
multi-line description.'''

[[dependencies.example]]
modId = "neoforge"
type = "required"
versionRange = "[21.1,)"
ordering = "NONE"
side = "BOTH"

[[dependencies.example]]
modId = "minecraft"
type = "required"
versionRange = "[1.21.1,1.22)"
"#;
        let v = parse(text).unwrap();
        assert_eq!(v.get_str("modLoader"), Some("javafml"));
        let mods = v.get("mods").unwrap().as_arr().unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].get_str("modId"), Some("example"));
        assert_eq!(mods[0].get_str("version"), Some("1.2.3"));
        assert!(mods[0].get_str("description").unwrap().contains("multi-line"));
        let deps = v.get("dependencies").unwrap().get("example").unwrap();
        let deps = deps.as_arr().unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[1].get_str("versionRange"), Some("[1.21.1,1.22)"));
    }

    #[test]
    fn keeps_placeholders_as_written() {
        let text = "[[mods]]\nmodId = \"x\"\nversion = \"${file.jarVersion}\"\n";
        let v = parse(text).unwrap();
        let mods = v.get("mods").unwrap().as_arr().unwrap();
        assert_eq!(mods[0].get_str("version"), Some("${file.jarVersion}"));
    }

    #[test]
    fn reads_quoted_and_dotted_tables() {
        let text = r#"
[dependencies."my.mod"]
modId = "my.mod"
versionRange = "[1.0]"
mandatory = true
"#;
        let v = parse(text).unwrap();
        let dep = v
            .get("dependencies")
            .unwrap()
            .get("my.mod")
            .unwrap();
        assert_eq!(dep.get_str("modId"), Some("my.mod"));
        assert_eq!(dep.get("mandatory").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn reads_arrays_across_lines_and_comments() {
        let text = r#"
[[mods]]
modId = "x"
authors = [
  "a",   # first
  "b",
]
"#;
        let v = parse(text).unwrap();
        let mods = v.get("mods").unwrap().as_arr().unwrap();
        assert_eq!(mods[0].get("authors").unwrap().as_arr().unwrap().len(), 2);
    }

    #[test]
    fn reports_a_truncated_header_by_line() {
        let error = parse("[[mods\nmodId = \"x\"").unwrap_err();
        assert_eq!(error.line, 1, "{error}");
        assert!(error.message.contains("`]]`"), "{error}");
    }

    #[test]
    fn refuses_an_unterminated_array() {
        let error = parse("authors = [\n  \"a\"").unwrap_err();
        assert!(error.message.contains("not closed"), "{error}");
    }

    #[test]
    fn strips_comments_but_not_inside_strings() {
        assert_eq!(strip_comment("a = \"x # y\" # tail"), "a = \"x # y\" ");
        assert_eq!(strip_comment("a = 'x # y'"), "a = 'x # y'");
    }

    #[test]
    fn values_round_trip_through_json() {
        let text = "n = 3\nd = 0.98\ns = \"quoted \\\"x\\\"\"\nb = false\n";
        let v = parse(text).unwrap();
        assert_eq!(v.get("n").unwrap().as_i64(), Some(3));
        assert_eq!(v.get("d").unwrap().as_f64(), Some(0.98));
        assert_eq!(v.get_str("s"), Some("quoted \"x\""));
        assert_eq!(v.get("b").unwrap().as_bool(), Some(false));
    }
}
