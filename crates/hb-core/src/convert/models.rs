//! Blockstate, block model and item model conversions.
//!
//! * **1.8.9 → 1.13+.** A 1.8.9 model names textures with `blocks/` and
//!   `items/` and its blockstate variants key on numeric metadata
//!   (`"type=0"`), which no later version reads.
//! * **1.21.1 → 1.21.4+.** Item models moved behind
//!   `assets/<ns>/items/<name>.json`, a definition whose `model` is
//!   `{"type": "minecraft:model", "model": "<ns>:item/<name>"}`. A jar with only
//!   `models/item/<name>.json` shows a missing model in 1.21.4+.
//! * **1.21.1 → 26.x.** A blockstate variant's `x`, `y`, `uvlock` and `weight`
//!   were read as strings by 1.21's Gson (`"y": "90"`, `"uvlock": "true"`);
//!   26.x's codec refuses them. `template_farmland` became
//!   `template_cube_bottom_top_indented` with `side`/`bottom` textures.

use super::{object, Note, Outcome};
use hb_json::{patch::Editor, Value};
use hb_versions::{Layout, Release};

/// Texture folder names 1.13 renamed, as `(old, new)` on a model's texture
/// reference.
const LEGACY_TEXTURE_FOLDERS: &[(&str, &str)] = &[
    ("blocks/", "block/"),
    ("items/", "item/"),
    // 1.8 named some textures after the block, not the folder+name pair.
    ("textures/blocks/", "block/"),
    ("textures/items/", "item/"),
];

pub fn blockstate(
    value: &mut Value,
    editor: &mut Editor,
    source: Layout,
    target: &'static Release,
    outcome: &mut Outcome,
    file: &str,
) {
    if target.layout == Layout::Legacy {
        outcome.note(
            Note::warn(
                "asset.blockstate.legacy_target",
                "1.8.9 reads no blockstate JSON: a mod's block model and states are Java code. The \
                 file is kept for reference only",
            )
            .at(file.to_owned()),
        );
        return;
    }
    // 1.21's Gson read numbers and booleans written as strings; 26.x's codec
    // does not, and skips the whole blockstate.
    if target.layout == Layout::ModernHorizon {
        let mut typed = 0;
        if let Some(variants) = value.get_mut("variants").and_then(Value::as_obj_mut) {
            for (key, variant) in variants.iter_mut() {
                typed += type_variant_values(variant, editor, &format!("variants/{}", hb_json::patch::join(&[key])));
            }
        }
        if let Some(multipart) = value.get_mut("multipart").and_then(Value::as_arr_mut) {
            for (i, part) in multipart.iter_mut().enumerate() {
                if let Some(apply) = part.get_mut("apply") {
                    typed += type_variant_values(apply, editor, &format!("multipart/{i}/apply"));
                }
                if let Some(when) = part.get_mut("when") {
                    typed += type_when_values(when, editor, &format!("multipart/{i}/when"));
                }
            }
        }
        if typed > 0 {
            outcome.note(
                Note::info(
                    "asset.blockstate.typed_values",
                    format!(
                        "{typed} string-encoded number(s)/boolean(s) became literals: 26.x's codec \
                         refuses `\"90\"` and `\"true\"` where 1.21's Gson accepted them"
                    ),
                )
                .at(file.to_owned()),
            );
        }
    }
    // Texture references inside the models this file points at are handled in
    // `model`; here we only rewrite the model names for a legacy source.
    if source == Layout::Legacy {
        let mut renamed = 0;
        if let Some(variants) = value.get_mut("variants").and_then(Value::as_obj_mut) {
            for (_, variant) in variants.iter_mut() {
                renamed += legacy_model_name(variant, editor);
            }
        }
        if let Some(multipart) = value.get_mut("multipart").and_then(Value::as_arr_mut) {
            for part in multipart.iter_mut() {
                if let Some(apply) = part.get_mut("apply") {
                    renamed += legacy_model_name(apply, editor);
                }
            }
        }
        if renamed > 0 {
            outcome.note(
                Note::info(
                    "asset.blockstate.legacy_names",
                    format!("{renamed} numeric/legacy model reference(s) rewritten to names"),
                )
                .at(file.to_owned()),
            );
        }
    }
}

/// Types `x`, `y`, `uvlock` and `weight` when they arrive as strings.
fn type_variant_values(variant: &mut Value, editor: &mut Editor, path: &str) -> usize {
    let mut typed = 0;
    match variant {
        Value::Obj(object) => {
            let keys: Vec<String> = object.keys().map(|k| k.to_owned()).collect();
            // A variant is `{"model": ..., "x": ..., ...}` or a list of those;
            // only the fields beside `model` are numbers, so decide that before
            // borrowing a field mutably.
            let has_model = object.contains_key("model");
            for key in keys {
                if key == "model" {
                    continue;
                }
                let Some(node) = object.get_mut(&key) else {
                    continue;
                };
                if has_model {
                    let before = node.clone();
                    if let Some(replacement) = literal_of(node) {
                        *node = replacement;
                        typed += 1;
                        editor.record(
                            "asset.blockstate.typed_value",
                            &format!("{path}/{}", hb_json::patch::join(&[&key])),
                            hb_json::patch::Node::Present(before),
                            hb_json::patch::Node::of(node),
                            "26.x reads these as JSON numbers and booleans, not strings",
                        );
                    }
                }
            }
        }
        Value::Arr(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                typed += type_variant_values(item, editor, &format!("{path}/{i}"));
            }
        }
        _ => {}
    }
    typed
}

/// `"90"` -> `90`, `"true"` -> `true`; `"east"` stays.
fn literal_of(value: &Value) -> Option<Value> {
    let text = value.as_str()?;
    match text {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        _ => {}
    }
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return None;
    }
    text.parse::<i64>().ok().map(Value::int)
}

/// A multipart `when` block's values: `{"x": "1", "down": "false"}`.
fn type_when_values(when: &mut Value, editor: &mut Editor, path: &str) -> usize {
    let mut typed = 0;
    if let Some(object) = when.as_obj_mut() {
        for key in ["AND", "OR"] {
            if let Some(items) = object.get_mut(key).and_then(Value::as_arr_mut) {
                for (i, item) in items.iter_mut().enumerate() {
                    typed += type_when_values(item, editor, &format!("{path}/{key}/{i}"));
                }
            }
        }
        let keys: Vec<String> = object.keys().map(|k| k.to_owned()).collect();
        for key in keys {
            if key == "AND" || key == "OR" {
                continue;
            }
            let Some(node) = object.get_mut(&key) else {
                continue;
            };
            let before = node.clone();
            match node {
                // A `when` value is a literal or a list of them:
                // `{"x": "1", "down": "false"}`. 26.x's codec reads numbers and
                // booleans, and refuses the quoted forms.
                Value::Arr(items) => {
                    let mut changed = false;
                    for item in items.iter_mut() {
                        if let Some(literal) = literal_of(item) {
                            *item = literal;
                            changed = true;
                        }
                    }
                    if changed {
                        typed += 1;
                    }
                }
                _ => match literal_of(node) {
                    Some(literal) => {
                        *node = literal;
                        typed += 1;
                    }
                    None => continue,
                },
            }
            editor.record(
                "asset.blockstate.typed_value",
                &format!("{path}/{}", hb_json::patch::join(&[&key])),
                hb_json::patch::Node::Present(before),
                hb_json::patch::Node::of(node),
                "26.x reads these as JSON booleans, not strings",
            );
        }
    }
    typed
}

/// Rewrites `"model": "blocks/stone"` to a namespaced name, and legacy numeric
/// references (`"model": "0"`) to a note (they cannot be converted).
fn legacy_model_name(variant: &mut Value, editor: &mut Editor) -> usize {
    let mut renamed = 0;
    // The model is one field in an object variant, one field two levels down in
    // a multipart `apply`, or one per item in a variant list. Find them by
    // path so that each can be written without holding two mutable borrows.
    let mut slots: Vec<Vec<String>> = Vec::new();
    match variant {
        Value::Obj(object) => {
            if object.contains_key("model") {
                slots.push(vec!["model".to_owned()]);
            }
            if let Some(apply) = object.get("apply") {
                if apply.has("model") {
                    slots.push(vec!["apply".to_owned(), "model".to_owned()]);
                }
            }
        }
        Value::Arr(items) => {
            for (i, item) in items.iter().enumerate() {
                if item.has("model") {
                    slots.push(vec![i.to_string(), "model".to_owned()]);
                }
            }
        }
        _ => {}
    }
    for slot in slots {
        let node = match slot.as_slice() {
            [field] => variant.get_mut(field),
            [first, field] => variant
                .get_mut(first)
                .and_then(|parent| parent.get_mut(field)),
            _ => None,
        };
        let Some(node) = node else {
            continue;
        };
        let Some(name) = node.as_str().map(|s| s.to_owned()) else {
            continue;
        };
        let name = name.as_str();
        if name.chars().all(|c| c.is_ascii_digit()) {
            // A numeric model id is a 1.8.9 runtime reference; nothing in the
            // file says which model it meant.
            continue;
        }
        let mut new = name.to_owned();
        for (from, to) in LEGACY_TEXTURE_FOLDERS {
            if let Some(rest) = new.strip_prefix(from) {
                new = format!("{to}{rest}");
            }
        }
        if new != name {
            *node = Value::str(new);
            renamed += 1;
            editor.record(
                "asset.model.legacy_name",
                "model",
                hb_json::patch::Node::Present(Value::str(name.to_owned())),
                hb_json::patch::Node::Present(Value::str(node.as_str().unwrap_or(""))),
                "1.13 flattened model and texture names",
            );
        }
    }
    renamed
}

pub fn model(
    value: &mut Value,
    editor: &mut Editor,
    source: Layout,
    target: &'static Release,
    outcome: &mut Outcome,
    file: &str,
    kind: super::ResourceKind,
    package_id: &str,
) {
    // Texture references: 1.8.9 wrote `blocks/` and `items/`.
    if source == Layout::Legacy {
        let mut renamed = 0;
        if let Some(textures) = value.get_mut("textures").and_then(Value::as_obj_mut) {
            let keys: Vec<String> = textures.keys().map(|k| k.to_owned()).collect();
            for key in keys {
                let Some(node) = textures.get_mut(&key) else {
                    continue;
                };
                let Some(name) = node.as_str().map(|s| s.to_owned()) else {
                    continue;
                };
                let name = name.as_str();
                let mut new = name.to_owned();
                for (from, to) in LEGACY_TEXTURE_FOLDERS {
                    if let Some(rest) = new.strip_prefix(from) {
                        new = format!("{to}{rest}");
                    }
                }
                // A texture reference with no namespace is the mod's own.
                if !new.contains(':') && !new.starts_with('#') {
                    new = format!("minecraft:{new}");
                }
                if new != name {
                    *node = Value::str(new.clone());
                    renamed += 1;
                    editor.record(
                        "asset.model.texture_renamed",
                        &format!("textures/{}", hb_json::patch::join(&[&key])),
                        hb_json::patch::Node::Present(Value::str(name.to_owned())),
                        hb_json::patch::Node::Present(Value::str(new)),
                        "1.13 moved block textures to `block/` and item textures to `item/`",
                    );
                }
            }
        }
        if renamed > 0 {
            outcome.note(
                Note::info(
                    "asset.model.textures",
                    format!("{renamed} legacy texture reference(s) rewritten"),
                )
                .at(file.to_owned()),
            );
        }
    }
    // 26.x's farmland template moved its dirt texture to `side` and `bottom`.
    if target.layout == Layout::ModernHorizon {
        let parent_is_farmland = value
            .get_str("parent")
            .map(|p| p.trim_start_matches("minecraft:"))
            .is_some_and(|p| p == "block/template_farmland");
        if parent_is_farmland {
            let before = value.clone();
            if let Some(object) = value.as_obj_mut() {
                object.insert(
                    "parent",
                    Value::str("minecraft:block/template_cube_bottom_top_indented"),
                );
                if let Some(textures) = object.get_mut("textures").and_then(Value::as_obj_mut) {
                    if let Some(dirt) = textures.get("dirt").cloned() {
                        if !textures.contains_key("side") {
                            textures.insert("side", dirt.clone());
                        }
                        if !textures.contains_key("bottom") {
                            textures.insert("bottom", dirt);
                        }
                    }
                }
            }
            editor.record(
                "asset.model.farmland_template",
                "parent",
                hb_json::patch::Node::Present(before),
                hb_json::patch::Node::of(value),
                "26.x renamed `template_farmland` and reads `side` and `bottom`",
            );
            outcome.note(
                Note::info(
                    "asset.model.farmland_template",
                    "`template_farmland` became `template_cube_bottom_top_indented` for 26.x",
                )
                .at(file.to_owned()),
            );
        }
    }
    // 1.21.4 moved item models behind `assets/<ns>/items/<name>.json`; without
    // that definition the game shows a missing model even though the model
    // itself is present.
    if kind == super::ResourceKind::ItemModel
        && !matches!(target.layout, Layout::Legacy | Layout::Flattened | Layout::SingularFolders)
    {
        write_item_definition(value, outcome, file, package_id);
    }
    // An item model with `overrides` is a 1.21.4+ item *definition* concern;
    // the overrides themselves move to the definition file, which is written
    // by the packaging step.
    if kind == super::ResourceKind::ItemModel && target.layout != Layout::Legacy {
        if value.has("overrides") {
            outcome.note(
                Note::warn(
                    "asset.model.item_overrides",
                    "this item model uses `overrides` (predicate-driven models). 1.21.4+ reads \
                     those from the `items/<name>.json` definition, which this tool writes with a \
                     plain `minecraft:model`; the predicates are not reproduced",
                )
                .at(file.to_owned()),
            );
        }
    }
}

/// Validates an existing `items/<name>.json` definition.
pub fn item_definition(
    value: &mut Value,
    _editor: &mut Editor,
    target: &Release,
    outcome: &mut Outcome,
    file: &str,
) {
    if target.layout == Layout::Legacy {
        outcome.note(
            Note::warn(
                "asset.item_definition.legacy_target",
                "1.8.9 has no item model definitions; an item's model comes from its Java class",
            )
            .at(file.to_owned()),
        );
        return;
    }
    let model = value.get("model");
    match model {
        Some(Value::Obj(inner)) => {
            if inner.get("type").is_none() {
                outcome.note(
                    Note::error(
                        "asset.item_definition.no_type",
                        "a definition's `model` object needs a `type` (usually \
                         `minecraft:model`); the game skips a definition without one",
                    )
                    .at(file.to_owned()),
                );
            }
        }
        Some(Value::Str(_)) => outcome.note(
            Note::error(
                "asset.item_definition.bare_model",
                "a definition's `model` must be an object with a `type`; a bare string is not read",
            )
            .at(file.to_owned()),
        ),
        None => outcome.note(
            Note::error(
                "asset.item_definition.no_model",
                "an item definition needs a `model`",
            )
            .at(file.to_owned()),
        ),
        _ => {}
    }
}

/// The definition file body 1.21.4+ needs for an item model.
pub fn item_definition_body(namespace: &str, name: &str) -> Value {
    object([
        (
            "model",
            object([
                ("type", Value::str("minecraft:model")),
                ("model", Value::str(format!("{namespace}:item/{name}"))),
            ]),
        ),
    ])
}

/// Writes `assets/<ns>/items/<name>.json`, the definition 1.21.4+ needs beside
/// an item model. The path is derived from `file`
/// (`assets/example/models/item/gem.json`); the definition names the model
/// explicitly so it does not depend on the model being found by convention.
fn write_item_definition(value: &Value, outcome: &mut Outcome, file: &str, package_id: &str) {
    let Some((root, namespace, tail)) = super::split_namespace(file) else {
        return;
    };
    // The definition goes in under the package id, like every other file of
    // the mod's own, and names the model the same way.
    let namespace = if namespace == "minecraft" { namespace } else { package_id };
    let Some(name) = tail
        .strip_prefix("models/item/")
        .and_then(|rest| rest.strip_suffix(".json"))
    else {
        return;
    };
    if name.contains('/') {
        return;
    }
    let model = format!("{namespace}:item/{name}");
    // An item with computer-generated models (`builtin/entity`, a block's own
    // model) keeps its own definition; only a plain model is described here.
    if value
        .get_str("parent")
        .is_some_and(|parent| parent.starts_with("builtin/"))
    {
        return;
    }
    let definition = Value::obj([(
        "model",
        Value::obj([
            ("type", Value::str("minecraft:model")),
            ("model", Value::str(model.clone())),
        ]),
    )]);
    let path = format!("{root}/{namespace}/items/{name}.json");
    if outcome.files.contains_key(&path) {
        return;
    }
    outcome.files.insert(
        path.clone(),
        hb_json::to_string_pretty(&definition).into_bytes(),
    );
    outcome.note(
        Note::info(
            "asset.item_definition.written",
            format!(
                "`{path}` written: 1.21.4+ reads an item's model through a definition, and an \
                 item model alone shows as a missing texture"
            ),
        )
        .at(file.to_owned()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_json::parse;

    fn target(id: &str) -> &'static Release {
        hb_versions::release(id).unwrap()
    }

    #[test]
    fn types_string_encoded_blockstate_values() {
        let mut v = parse(
            r#"{"variants":{"facing=north":{"model":"example:block/x","y":"90","uvlock":"true"}}}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        blockstate(
            &mut v,
            &mut editor,
            Layout::ItemDefinitions,
            target("26.3"),
            &mut outcome,
            "f.json",
        );
        let variant = &v.get("variants").unwrap().get("facing=north").unwrap().clone();
        assert_eq!(variant.get("y").unwrap().as_i64(), Some(90));
        assert_eq!(variant.get("uvlock").unwrap().as_bool(), Some(true));
        assert_eq!(
            variant.get_str("model"),
            Some("example:block/x"),
            "strings that are not literals are left alone"
        );
    }

    #[test]
    fn types_multipart_when_values() {
        let mut v = parse(
            r#"{"multipart":[{"apply":{"model":"t:block/cable"},"when":{"x":"1","down":"false"}}]}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        blockstate(
            &mut v,
            &mut editor,
            Layout::ItemDefinitions,
            target("26.3"),
            &mut outcome,
            "f.json",
        );
        let when = v.get("multipart").unwrap().as_arr().unwrap()[0]
            .get("when")
            .unwrap()
            .clone();
        assert_eq!(when.get("x").unwrap().as_i64(), Some(1));
        assert_eq!(when.get("down").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn rewrites_legacy_texture_references() {
        let mut v = parse(r#"{"parent":"builtin/generated","textures":{"layer0":"items/gem"}}"#).unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        model(
            &mut v,
            &mut editor,
            Layout::Legacy,
            target("1.21.1"),
            &mut outcome,
            "f.json",
            super::super::ResourceKind::ItemModel,
            "example",
        );
        assert_eq!(
            v.get("textures").unwrap().get_str("layer0"),
            Some("minecraft:item/gem")
        );
    }

    #[test]
    fn rewrites_the_farmland_template() {
        let mut v = parse(
            r#"{"parent":"minecraft:block/template_farmland","textures":{"dirt":"minecraft:block/dirt","top":"minecraft:block/farmland"}}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        model(
            &mut v,
            &mut editor,
            Layout::ItemDefinitions,
            target("26.3"),
            &mut outcome,
            "f.json",
            super::super::ResourceKind::BlockModel,
            "example",
        );
        assert_eq!(
            v.get_str("parent"),
            Some("minecraft:block/template_cube_bottom_top_indented")
        );
        let textures = v.get("textures").unwrap();
        assert_eq!(textures.get_str("side"), Some("minecraft:block/dirt"));
        assert_eq!(textures.get_str("bottom"), Some("minecraft:block/dirt"));
    }

    #[test]
    fn writes_item_definitions_in_the_documented_shape() {
        let body = item_definition_body("ruby", "ruby");
        assert_eq!(
            hb_json::to_string(&body),
            r#"{"model":{"type":"minecraft:model","model":"ruby:item/ruby"}}"#
        );
    }

    #[test]
    fn refuses_legacy_targets_with_a_reason() {
        let mut v = parse(r#"{"variants":{}}"#).unwrap();
        let mut editor = Editor::new();
        let mut outcome = Outcome::default();
        blockstate(
            &mut v,
            &mut editor,
            Layout::ItemDefinitions,
            target("1.8.9"),
            &mut outcome,
            "f.json",
        );
        assert_eq!(outcome.warnings().count(), 1);
    }
}
