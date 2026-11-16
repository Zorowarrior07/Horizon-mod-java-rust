//! Language files.
//!
//! 1.6–1.12 used `assets/<ns>/lang/en_US.lang`, a `key=value` text file with
//! `%s`/`%d` format specifiers and `\n` escapes. 1.13+ reads
//! `assets/<ns>/lang/en_us.json` (lowercase locale) with `%s` replaced by
//! positional `%1$s`-style specifiers. Both are carried because a resource
//! pack may legitimately contain either.

use super::{Note, Outcome};
use hb_json::{patch::Editor, Object, Value};

/// Converts a legacy `.lang` file to the JSON form 1.13+ reads.
pub fn convert_legacy(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = decode_text(bytes)?;
    let mut object = Object::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `key=value`; a value may contain `=`.
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!(
                "line {}: no `=`, so it is not a key=value pair: `{}`",
                number + 1,
                truncate(line, 80)
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {}: empty key", number + 1));
        }
        object.insert(key, Value::str(unescape_lang(value)));
    }
    Ok(hb_json::to_string_pretty(&Value::Obj(object)).into_bytes())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max).collect::<String>() + "…"
    }
}

/// `.lang` files are UTF-8 in practice; Latin-1 appears in old packs.
fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(_) => {
            // Fall back to Latin-1, which cannot fail, and say nothing: the
            // bytes become the characters they were.
            Ok(bytes.iter().map(|&b| b as char).collect())
        }
    }
}

fn unescape_lang(value: &str) -> String {
    // `.lang` values are taken literally apart from `\n`, as the 1.8.9 reader
    // did not process other escapes.
    value.replace("\\n", "\n")
}

/// Checks a modern language file: it must be a flat object of strings, and a
/// `%s` in a value is a 1.13+ format error worth reporting.
pub fn modern(value: &mut Value, editor: &mut Editor, outcome: &mut Outcome, file: &str) {
    let Some(object) = value.as_obj() else {
        outcome.note(
            Note::error(
                "asset.lang.not_an_object",
                "a language file must be a JSON object of `key: text`",
            )
            .at(file.to_owned()),
        );
        return;
    };
    let mut non_string = Vec::new();
    let mut unpositioned = Vec::new();
    for (key, value) in object.iter() {
        match value {
            Value::Str(text) => {
                // 1.13+ moved `%s` to `%1$s`; a bare `%s` no longer formats.
                if text.contains("%s") && !text.contains("%1$s") {
                    unpositioned.push(key.to_owned());
                }
            }
            _ => non_string.push(key.to_owned()),
        }
    }
    if !non_string.is_empty() {
        outcome.note(
            Note::warn(
                "asset.lang.non_string",
                format!(
                    "{} value(s) are not strings ({}); the game skips the whole file",
                    non_string.len(),
                    non_string.join(", ")
                ),
            )
            .at(file.to_owned()),
        );
    }
    if !unpositioned.is_empty() {
        // Rewriting `%s` to `%1$s` requires knowing how many arguments the
        // string takes and in what order; that is mechanical only when there is
        // exactly one, which is the common case.
        let keys: Vec<String> = unpositioned.clone();
        for key in keys {
            if let Some(Value::Str(text)) = value.get_mut(&key) {
                if text.matches("%s").count() == 1 && !text.contains("%d") {
                    let new = text.replace("%s", "%1$s");
                    editor.set(
                        value,
                        "asset.lang.positional_format",
                        &hb_json::patch::join(&[&key]),
                        Value::str(new),
                        "1.13+ formats with positional specifiers; a bare `%s` throws and the line \
                         shows the raw key",
                    );
                } else {
                    outcome.note(
                        Note::warn(
                            "asset.lang.unpositioned_format",
                            format!(
                                "`{key}` uses a bare `%s` among other arguments: it needs \
                                 positional specifiers (`%1$s`, `%2$s`) written by hand"
                            ),
                        )
                        .at(file.to_owned()),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_legacy_lang_file() {
        let bytes = b"# comment\nitem.example.gem=Gem\nblock.example.ore=Ore \\n shiny\n\
                      tooltip.example=100%% pure\n";
        let json = convert_legacy(bytes).unwrap();
        let value = hb_json::parse_bytes(&json).unwrap();
        assert_eq!(value.get_str("item.example.gem"), Some("Gem"));
        assert_eq!(
            value.get_str("block.example.ore"),
            Some("Ore \n shiny"),
            "\\n becomes a newline"
        );
        assert_eq!(value.get_str("tooltip.example"), Some("100%% pure"));
        // Keys are the 1.8.9 forms, which are still the keys 1.13+ uses for
        // `item.<ns>.<name>` and `block.<ns>.<name>`.
        assert!(hb_json::parse_bytes(&json).is_ok());
    }

    #[test]
    fn refuses_a_lang_file_that_is_not_key_value() {
        assert!(convert_legacy(b"just a sentence\n").unwrap_err().contains("no `=`"));
    }

    #[test]
    fn converts_a_single_bare_format_specifier() {
        let mut value = hb_json::parse(r#"{"a":"Hello %s","b":"%s of %s"}"#).unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        modern(&mut value, &mut editor, &mut outcome, "en_us.json");
        assert_eq!(value.get_str("a"), Some("Hello %1$s"));
        assert_eq!(value.get_str("b"), Some("%s of %s"), "ambiguous, left alone");
        assert_eq!(outcome.warnings().count(), 1);
    }

    #[test]
    fn reports_non_string_values() {
        let mut value = hb_json::parse(r#"{"a":1}"#).unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        modern(&mut value, &mut editor, &mut outcome, "en_us.json");
        assert!(outcome
            .warnings()
            .any(|w| w.code == "asset.lang.non_string"));
    }
}
