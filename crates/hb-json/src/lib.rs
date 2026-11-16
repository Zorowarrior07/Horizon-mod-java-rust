//! `hb-json`: a small, strict, dependency-free JSON implementation tailored to
//! Minecraft data.
//!
//! Design goals, in order:
//!
//! 1. **Round-trip fidelity for numbers.** Minecraft's Gson reads a bare `4`
//!    as an int and `4.0` as a float; pack authors notice when `4.0` becomes
//!    `4`. Numbers therefore keep the exact text they were read from and are
//!    re-emitted unchanged unless a conversion deliberately rewrites them.
//! 2. **Insertion order.** Objects keep the order keys were read in, so a
//!    converted file stays diffable against the original. Lookup is a linear
//!    scan; Minecraft JSON objects have a handful of keys, not thousands.
//! 3. **Strictness.** Duplicate keys, trailing commas, comments, `NaN`, lone
//!    surrogates and control characters are refused with a byte offset. A
//!    converter that silently accepted those would hide mod author mistakes.
//! 4. **No panics.** Every public entry point returns `Result`.
//!
//! A [`patch`] engine applies named, reversible edits so a conversion report
//! can say exactly what changed rather than "converted".

pub mod number;
pub mod patch;

use std::fmt::Write as _;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(number::Number),
    Str(String),
    Arr(Vec<Value>),
    /// Insertion-ordered members. See [`Object`].
    Obj(Object),
}

/// An insertion-ordered JSON object.
///
/// Small by nature (a blockstate variant key, a recipe body), so a `Vec` of
/// pairs beats a hash map both in memory and in determinism.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Object {
    entries: Vec<(String, Value)>,
}

impl Object {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self { entries: Vec::with_capacity(n) }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Inserts, keeping the position of an existing key. Returns the previous
    /// value, if any. This is the Minecraft-friendly behaviour: a conversion
    /// that rewrites `"hardness"` should not move it to the end.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        let key = key.into();
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            return Some(std::mem::replace(&mut slot.1, value));
        }
        self.entries.push((key, value));
        None
    }

    /// Inserts, moving an existing key to the end. Used when a key's meaning
    /// changes so that a reader sees it where the new format expects it.
    pub fn insert_last(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        let key = key.into();
        let previous = self.remove(&key);
        self.entries.push((key, value));
        previous
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(at).1)
    }

    /// Renames a key in place, keeping its position. If `to` already exists,
    /// the old entry is removed and the existing `to` is kept.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let Some(at) = self.entries.iter().position(|(k, _)| k == from) else {
            return false;
        };
        if from == to {
            return false;
        }
        if self.entries.iter().any(|(k, _)| k == to) {
            self.entries.remove(at);
        } else {
            self.entries[at].0 = to.to_owned();
        }
        true
    }

    /// Sorted view of the keys, for diagnostics and stable reports.
    pub fn sorted_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.keys().collect();
        keys.sort_unstable();
        keys
    }
}

impl FromIterator<(String, Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        let mut object = Object::new();
        for (k, v) in iter {
            object.insert(k, v);
        }
        object
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Object> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_obj_mut(&mut self) -> Option<&mut Object> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_arr_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// The value as an `i64` when it is a whole number that fits.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) => n.as_i64(),
            _ => None,
        }
    }

    /// The value as an `f64` when it is a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => n.as_f64(),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_obj()?.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Convenience for optional-presence checks used all over the converters.
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn is_obj(&self) -> bool {
        matches!(self, Value::Obj(_))
    }

    pub fn is_arr(&self) -> bool {
        matches!(self, Value::Arr(_))
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    /// A one-line, human-readable kind name for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Arr(_) => "array",
            Value::Obj(_) => "object",
        }
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn int(n: impl Into<i64>) -> Value {
        Value::Num(number::Number::from_i64(n.into()))
    }

    pub fn float(f: f64) -> Value {
        Value::Num(number::Number::from_f64(f))
    }

    pub fn obj(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    pub fn arr(items: impl IntoIterator<Item = Value>) -> Value {
        Value::Arr(items.into_iter().collect())
    }
}

/// Errors from [`parse`], with the byte offset they were found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}, column {} (byte {}): {}",
            self.line, self.column, self.offset, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Configuration for [`parse_with`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum nesting depth. Vanilla data nests shallowly; a depth bomb is a
    /// corrupt or hostile file.
    pub max_depth: usize,
    /// Maximum input length in bytes.
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_bytes: 64 << 20,
        }
    }
}

/// Parses a complete JSON document. Surrounding whitespace is allowed; any
/// trailing non-whitespace is refused.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    parse_with(text, Limits::default())
}

pub fn parse_bytes(bytes: &[u8]) -> Result<Value, ParseError> {
    // Minecraft JSON is UTF-8. A BOM is tolerated (Windows editors add one);
    // Gson tolerates it too.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let at = e.valid_up_to();
        let (line, column) = line_col(bytes, at);
        ParseError {
            offset: at,
            line,
            column,
            message: "not valid UTF-8".to_owned(),
        }
    })?;
    parse_with(text, Limits::default())
}

pub fn parse_with(text: &str, limits: Limits) -> Result<Value, ParseError> {
    if text.len() > limits.max_bytes {
        return Err(ParseError {
            offset: 0,
            line: 1,
            column: 1,
            message: format!(
                "document is {} bytes, over the {} byte limit",
                text.len(),
                limits.max_bytes
            ),
        });
    }
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
        limits,
    };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.at < parser.bytes.len() {
        return Err(parser.error("trailing characters after the document"));
    }
    Ok(value)
}

fn line_col(bytes: &[u8], offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for &b in &bytes[..offset.min(bytes.len())] {
        if b == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    limits: Limits,
}

impl<'a> Parser<'a> {
    fn error(&self, message: impl Into<String>) -> ParseError {
        let (line, column) = line_col(self.bytes, self.at);
        ParseError {
            offset: self.at,
            line,
            column,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.at += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), ParseError> {
        match self.peek() {
            Some(c) if c == b => {
                self.at += 1;
                Ok(())
            }
            Some(c) => Err(self.error(format!(
                "expected `{}`, found `{}`",
                b as char,
                (c as char).escape_default()
            ))),
            None => Err(self.error(format!(
                "expected `{}`, found end of input",
                b as char
            ))),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > self.limits.max_depth {
            return Err(self.error(format!(
                "nesting deeper than {} levels",
                self.limits.max_depth
            )));
        }
        match self.peek() {
            None => Err(self.error("expected a value, found end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(self.error(format!(
                "unexpected `{}` where a value was expected",
                (c as char).escape_default()
            ))),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.expect(b'{')?;
        let mut object = Object::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Obj(object));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a quoted key"));
            }
            let key_at = self.at;
            let key = self.string()?;
            if object.contains_key(&key) {
                self.at = key_at;
                return Err(self.error(format!("duplicate key `{key}`")));
            }
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            object.entries.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => break,
                Some(c) => {
                    self.at -= 1;
                    return Err(self.error(format!(
                        "expected `,` or `}}`, found `{}`",
                        (c as char).escape_default()
                    )));
                }
                None => return Err(self.error("unterminated object")),
            }
        }
        Ok(Value::Obj(object))
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => break,
                Some(c) => {
                    self.at -= 1;
                    return Err(self.error(format!(
                        "expected `,` or `]`, found `{}`",
                        (c as char).escape_default()
                    )));
                }
                None => return Err(self.error("unterminated array")),
            }
        }
        Ok(Value::Arr(items))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(b) = self.bump() else {
                return Err(self.error("unterminated string"));
            };
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(e) = self.bump() else {
                        return Err(self.error("unterminated escape"));
                    };
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            if (0xD800..0xDC00).contains(&unit) {
                                // High surrogate: a low surrogate must follow.
                                if self.bytes.get(self.at) != Some(&b'\\')
                                    || self.bytes.get(self.at + 1) != Some(&b'u')
                                {
                                    return Err(self.error(
                                        "lone high surrogate in \\u escape (Java would keep it, \
                                         but Minecraft text breaks on it)",
                                    ));
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("high surrogate not followed by a low one"));
                                }
                                let code = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                                let ch = char::from_u32(code)
                                    .ok_or_else(|| self.error("invalid surrogate pair"))?;
                                out.push(ch);
                            } else if (0xDC00..0xE000).contains(&unit) {
                                return Err(self.error("lone low surrogate in \\u escape"));
                            } else {
                                out.push(
                                    char::from_u32(unit)
                                        .ok_or_else(|| self.error("invalid \\u escape"))?,
                                );
                            }
                        }
                        other => {
                            return Err(self.error(format!(
                                "unknown escape `\\{}`",
                                (other as char).escape_default()
                            )))
                        }
                    }
                }
                0x00..=0x1F => {
                    return Err(self.error(format!(
                        "raw control character U+{:04X} in a string",
                        b
                    )))
                }
                0x20..=0x7F => out.push(b as char),
                _ => {
                    // Multi-byte UTF-8: re-read the sequence from the source.
                    let start = self.at - 1;
                    let len = utf8_len(b);
                    if start + len > self.bytes.len() {
                        return Err(self.error("truncated UTF-8 sequence"));
                    }
                    let slice = &self.bytes[start..start + len];
                    let s = std::str::from_utf8(slice)
                        .map_err(|_| self.error("invalid UTF-8 in a string"))?;
                    out.push_str(s);
                    self.at = start + len;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(b) = self.bump() else {
                return Err(self.error("truncated \\u escape"));
            };
            let digit = (b as char)
                .to_digit(16)
                .ok_or_else(|| self.error("invalid hex digit in \\u escape"))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        // Integer part: `0` alone, or a non-zero digit then digits.
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.at = start;
                    return Err(self.error("leading zero in a number"));
                }
            }
            Some(c) if c.is_ascii_digit() => {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.at += 1;
                }
            }
            _ => {
                return Err(self.error("expected a digit"));
            }
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.error("expected a digit after the decimal point"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.error("expected a digit in the exponent"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        let text = &self.bytes[start..self.at];
        // SAFETY-free: the slice is ASCII by construction.
        let text = std::str::from_utf8(text).map_err(|_| self.error("invalid number"))?;
        Ok(Value::Num(number::Number::parse_unchecked(text)))
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Serialises compactly (no whitespace), which is what `.hmod` files and
/// `--json` reports want.
pub fn to_string(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, None, 0);
    out
}

/// Pretty-prints with two-space indentation and a trailing newline, the style
/// Minecraft's own data files use and `git diff` likes.
pub fn to_string_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, Some(2), 0);
    out.push('\n');
    out
}

fn write_indent(out: &mut String, indent: Option<usize>, depth: usize) {
    if let Some(width) = indent {
        out.push('\n');
        for _ in 0..width * depth {
            out.push(' ');
        }
    }
}

fn write_value(out: &mut String, value: &Value, indent: Option<usize>, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Num(n) => out.push_str(&n.to_string()),
        Value::Str(s) => write_string(out, s),
        Value::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_indent(out, indent, depth + 1);
                write_value(out, item, indent, depth + 1);
            }
            write_indent(out, indent, depth);
            out.push(']');
        }
        Value::Obj(object) => {
            if object.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in object.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_indent(out, indent, depth + 1);
                write_string(out, key);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(out, item, indent, depth + 1);
            }
            write_indent(out, indent, depth);
            out.push('}');
        }
    }
}

/// Writes a JSON string literal. Everything outside printable ASCII is
/// escaped, matching Gson's default `toJson` output, so the bytes are pure
/// ASCII and a Windows editor cannot mangle them.
fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            other => {
                let mut buf = [0u16; 2];
                for unit in other.encode_utf16(&mut buf) {
                    // Writing into a String cannot fail.
                    let _ = write!(out, "\\u{:04x}", unit);
                }
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_gamut() {
        let v = parse(r#"{"a":[1,2.5,-3e2,true,false,null],"b":"x"}"#).unwrap();
        assert_eq!(v.get("b").unwrap().as_str(), Some("x"));
        let a = v.get("a").unwrap().as_arr().unwrap();
        assert_eq!(a[0].as_i64(), Some(1));
        assert_eq!(a[1].as_f64(), Some(2.5));
        assert_eq!(a[2].as_f64(), Some(-300.0));
    }

    #[test]
    fn keeps_number_text_exactly() {
        // The bug this guards: 4.0 read as a float and written back as 4
        // changes a block's hardness from float to int in Gson's eyes.
        for text in ["4.0", "0.98", "1e2", "-0.0", "123456789012345678901234567890"] {
            let v = parse(text).unwrap();
            assert_eq!(to_string(&v), text, "{text}");
        }
    }

    #[test]
    fn keeps_insertion_order_through_edits() {
        let mut v = parse(r#"{"z":1,"a":2,"m":3}"#).unwrap();
        v.as_obj_mut().unwrap().insert("a", Value::int(9));
        assert_eq!(to_string(&v), r#"{"z":1,"a":9,"m":3}"#);
        v.as_obj_mut().unwrap().insert("b", Value::int(4));
        assert_eq!(to_string(&v), r#"{"z":1,"a":9,"m":3,"b":4}"#);
    }

    #[test]
    fn refuses_malformed_input_with_a_position() {
        for (text, needle) in [
            (r#"{"a":1,}"#, "expected a quoted key"),
            (r#"{"a" 1}"#, "expected `:`"),
            ("[1,]", "unexpected `]`"),
            (r#"{"a":1}{"b":2}"#, "trailing characters"),
            (r#"{"a":1,"a":2}"#, "duplicate key"),
            ("01", "leading zero"),
            ("nul", "expected `null`"),
            (r#""\x""#, "unknown escape"),
            ("\"a\nb\"", "raw control character"),
        ] {
            let err = parse(text).unwrap_err();
            assert!(
                err.message.contains(needle),
                "{text:?} -> {err} (wanted {needle:?})"
            );
        }
    }

    #[test]
    fn refuses_depth_bombs() {
        let deep = "[".repeat(500) + &"]".repeat(500);
        assert!(parse(&deep).unwrap_err().message.contains("nesting"));
    }

    #[test]
    fn round_trips_non_ascii_and_escapes() {
        let text = r#"{"n":"caf\u00e9 \ud83d\ude00 \"q\" \\"}"#;
        let v = parse(text).unwrap();
        assert_eq!(
            v.get("n").unwrap().as_str(),
            Some("café 😀 \"q\" \\"),
            "escapes decode"
        );
        assert_eq!(
            to_string(&v),
            r#"{"n":"caf\u00e9 \ud83d\ude00 \"q\" \\"}"#,
            "non-ASCII re-escapes to ASCII, as Gson does"
        );
        // And a round trip through our own output is stable.
        assert_eq!(to_string(&parse(&to_string(&v)).unwrap()), to_string(&v));
    }

    #[test]
    fn tolerates_a_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"{\"a\":1}");
        assert_eq!(parse_bytes(&bytes).unwrap().get("a").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn pretty_prints_stably() {
        let v = parse(r#"{"b":[],"a":{"c":1}}"#).unwrap();
        assert_eq!(
            to_string_pretty(&v),
            "{\n  \"b\": [],\n  \"a\": {\n    \"c\": 1\n  }\n}\n"
        );
    }

    #[test]
    fn rename_keeps_position_and_handles_collisions() {
        let mut v = parse(r#"{"a":1,"b":2,"c":3}"#).unwrap();
        assert!(v.as_obj_mut().unwrap().rename("b", "bb"));
        assert_eq!(to_string(&v), r#"{"a":1,"bb":2,"c":3}"#);
        // Renaming onto an existing key drops the renamed entry, keeping the
        // existing one where it was: `bb` then `c`.
        assert!(v.as_obj_mut().unwrap().rename("a", "c"));
        assert_eq!(to_string(&v), r#"{"bb":2,"c":3}"#);
    }
}
