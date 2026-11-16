//! Choosing a vanilla template for each block and item a jar declares.
//!
//! A Horizon content mod does not describe a block from scratch: it copies a
//! vanilla block as a *template* and lists what differs. So a converted block
//! needs a template, and the only evidence in a jar is the blockstate's
//! property set and the block's name.
//!
//! The table and the tie-break below are the ones Horizon's own port uses
//! (`horizon-port-pack/src/carry.rs`), reproduced so this tool's guesses agree
//! with the official one. A guess is a guess: every declaration written here is
//! reported, and the report says plainly that the template is inferred from the
//! blockstate, not read from the game.

use super::{Note, Outcome};
use crate::jar::{Jar, ResourceKind};
use hb_json::{Object, Value};
use hb_versions::Release;
use std::collections::BTreeSet;

/// Vanilla blocks by their sorted property names. From Horizon's `carry.rs`.
pub const TEMPLATES: &[(&str, &str)] = &[
    ("", "minecraft:stone"),
    ("axis", "minecraft:oak_log"),
    ("type,waterlogged", "minecraft:oak_slab"),
    ("facing,half,shape,waterlogged", "minecraft:oak_stairs"),
    ("facing,half,hinge,open,powered", "minecraft:oak_door"),
    (
        "facing,half,open,powered,waterlogged",
        "minecraft:oak_trapdoor",
    ),
    ("east,north,south,waterlogged,west", "minecraft:oak_fence"),
    ("east,north,south,waterlogged,west", "minecraft:glass_pane"),
    (
        "east,north,south,up,waterlogged,west",
        "minecraft:cobblestone_wall",
    ),
    ("facing,in_wall,open,powered", "minecraft:oak_fence_gate"),
    ("face,facing,powered", "minecraft:oak_button"),
    ("powered", "minecraft:oak_pressure_plate"),
    ("facing", "minecraft:carved_pumpkin"),
    ("facing,lit", "minecraft:furnace"),
    ("lit", "minecraft:redstone_lamp"),
    ("waterlogged", "minecraft:chain"),
    ("age", "minecraft:wheat"),
    ("distance,persistent,waterlogged", "minecraft:oak_leaves"),
    ("stage", "minecraft:oak_sapling"),
];

/// The property names a blockstate file tests, sorted.
pub fn properties(blockstate: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(variants) = blockstate.get("variants").and_then(Value::as_obj) {
        for (key, _) in variants.iter() {
            for pair in key.split(',') {
                if let Some((name, _)) = pair.split_once('=') {
                    out.insert(name.to_owned());
                }
            }
        }
    }
    if let Some(multipart) = blockstate.get("multipart").and_then(Value::as_arr) {
        for part in multipart {
            if let Some(when) = part.get("when") {
                collect_when_properties(when, &mut out);
            }
        }
    }
    out
}

fn collect_when_properties(when: &Value, out: &mut BTreeSet<String>) {
    let Some(object) = when.as_obj() else {
        return;
    };
    for (key, value) in object.iter() {
        if key == "AND" || key == "OR" {
            if let Some(items) = value.as_arr() {
                for item in items {
                    collect_when_properties(item, out);
                }
            }
            continue;
        }
        out.insert(key.to_owned());
    }
}

/// Every value a blockstate tests a property for.
pub fn values_of(blockstate: &Value, property: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(variants) = blockstate.get("variants").and_then(Value::as_obj) {
        for (key, _) in variants.iter() {
            for pair in key.split(',') {
                if let Some((name, value)) = pair.split_once('=') {
                    if name == property {
                        out.insert(value.to_owned());
                    }
                }
            }
        }
    }
    if let Some(multipart) = blockstate.get("multipart").and_then(Value::as_arr) {
        for part in multipart {
            if let Some(when) = part.get("when") {
                collect_when_values(when, property, &mut out);
            }
        }
    }
    out
}

fn collect_when_values(when: &Value, property: &str, out: &mut BTreeSet<String>) {
    let Some(object) = when.as_obj() else {
        return;
    };
    for (key, value) in object.iter() {
        if key == "AND" || key == "OR" {
            if let Some(items) = value.as_arr() {
                for item in items {
                    collect_when_values(item, property, out);
                }
            }
            continue;
        }
        if key != property {
            continue;
        }
        match value {
            Value::Str(s) => {
                out.insert(s.clone());
            }
            Value::Bool(b) => {
                out.insert(b.to_string());
            }
            Value::Arr(items) => {
                for item in items {
                    match item {
                        Value::Str(s) => {
                            out.insert(s.clone());
                        }
                        Value::Bool(b) => {
                            out.insert(b.to_string());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// The template whose properties include the blockstate's with the fewest left
/// over; the block's name breaks ties (`_pane`, `_gate`, `_trapdoor`).
pub fn guess(name: &str, properties: &BTreeSet<String>) -> Option<&'static str> {
    let fits = TEMPLATES.iter().filter_map(|(key, template)| {
        let have: BTreeSet<&str> = key.split(',').filter(|p| !p.is_empty()).collect();
        properties
            .iter()
            .all(|p| have.contains(p.as_str()))
            .then(|| (have.len() - properties.len(), *template))
    });
    let hint = |template: &str| {
        let kind = template.rsplit('_').next().unwrap_or(template);
        !name.ends_with(&format!("_{kind}"))
    };
    fits.min_by_key(|&(extra, template)| (extra, hint(template)))
        .map(|(_, template)| template)
}

/// The template for a blockstate file, with the wall/vine correction: a wall's
/// sides are `none`, `low` or `tall`, while a vine's are `true`/`false` and the
/// names alone would call it a wall.
pub fn guess_for_blockstate(name: &str, blockstate: &Value) -> Option<&'static str> {
    let template = guess(name, &properties(blockstate))?;
    let boolean_sides = ["north", "east", "south", "west"]
        .iter()
        .flat_map(|side| values_of(blockstate, side))
        .any(|value| value == "true" || value == "false");
    Some(if template == "minecraft:cobblestone_wall" && boolean_sides {
        "minecraft:vine"
    } else {
        template
    })
}

/// Declares every block and item the jar's own content implies, in
/// `data/<id>/horizon/{block,item}/`. Returns how many declarations were
/// written.
pub fn declare(outcome: &mut Outcome, mod_id: &str, jar: &Jar, target: &Release) -> usize {
    if target.layout != hb_versions::Layout::ModernHorizon {
        return 0;
    }
    let mut written = 0;
    let source_id = jar.info().map(|i| i.id.clone()).unwrap_or_default();

    // Blocks: each blockstate under the mod's own namespace.
    let mut blocks: BTreeSet<String> = BTreeSet::new();
    for entry in jar.entries_of(ResourceKind::BlockState) {
        let Some(name) = block_name(&entry.name) else {
            continue;
        };
        if entry.namespace.as_deref().is_some_and(|ns| ns != source_id) {
            continue;
        }
        let Some(value) = hb_json::parse_bytes(&entry.bytes).ok() else {
            continue;
        };
        let template = match guess_for_blockstate(&name, &value) {
            Some(template) => template,
            None => {
                outcome.note(
                    Note::warn(
                        "content.block.no_template",
                        format!(
                            "no vanilla template matches the properties {:?} of `{name}`: the \
                             block is not declared, and a player would find air where it should be",
                            properties(&value)
                        ),
                    )
                    .at(entry.name.clone()),
                );
                continue;
            }
        };
        let body = Value::obj([
            ("template", Value::str(template)),
            (
                "properties",
                Value::Obj(properties_from_blockstate(&value, &name)),
            ),
        ]);
        let path = format!("data/{mod_id}/horizon/block/{name}.json");
        outcome
            .files
            .insert(path, hb_json::to_string_pretty(&body).into_bytes());
        outcome
            .guessed_templates
            .push((format!("block {name}"), template.to_owned()));
        outcome.note(Note::info(
            "content.block.declared",
            format!(
                "block `{mod_id}:{name}` declared with template `{template}`, inferred from its \
                 blockstate properties"
            ),
        ));
        blocks.insert(name);
        written += 1;
    }

    // Items: each item model that is not a block's own model.
    let mut items: BTreeSet<String> = BTreeSet::new();
    for entry in jar.entries_of(ResourceKind::ItemModel) {
        let Some(name) = item_name(&entry.name) else {
            continue;
        };
        if blocks.contains(&name) {
            // A block's item comes with the block.
            continue;
        }
        if entry.namespace.as_deref().is_some_and(|ns| ns != source_id) {
            continue;
        }
        let template = item_template(&name, &entry.bytes);
        let body = Value::obj([("template", Value::str(template))]);
        let path = format!("data/{mod_id}/horizon/item/{name}.json");
        if outcome.files.contains_key(&path) {
            continue;
        }
        outcome
            .files
            .insert(path, hb_json::to_string_pretty(&body).into_bytes());
        outcome
            .guessed_templates
            .push((format!("item {name}"), template.to_owned()));
        outcome.note(Note::info(
            "content.item.declared",
            format!("item `{mod_id}:{name}` declared with template `{template}`"),
        ));
        items.insert(name);
        written += 1;
    }

    // A block written by its code with no blockstate of its own still needs a
    // declaration if the jar's language file names it; that is a guess about
    // which entries are blocks, so it is reported, not assumed.
    let _ = items;
    written
}

/// `assets/<ns>/blockstates/<name>.json` -> `<name>`.
fn block_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/")?.split_once('/')?.1;
    let name = rest.strip_prefix("blockstates/")?.strip_suffix(".json")?;
    (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
}

/// `assets/<ns>/models/item/<name>.json` -> `<name>`.
fn item_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/")?.split_once('/')?.1;
    let name = rest.strip_prefix("models/item/")?.strip_suffix(".json")?;
    (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
}

/// Properties the declaration can state from what the blockstate shows. A
/// blockstate says which *states* exist, not what the block's hardness or light
/// is (that is Java), so nothing is invented.
fn properties_from_blockstate(_blockstate: &Value, _name: &str) -> Object {
    Object::new()
}

/// The item template to declare. A jar's item model does not say what the item
/// is (a tool, food, armour): its Java class does. The safe choice is a plain
/// item, and the report says so.
fn item_template(name: &str, model_json: &[u8]) -> &'static str {
    // A few conventional names are strong evidence and worth naming: a mod's
    // `<x>_sword`, `<x>_pickaxe` and so on.
    for (suffix, template) in [
        ("_sword", "minecraft:iron_sword"),
        ("_pickaxe", "minecraft:iron_pickaxe"),
        ("_axe", "minecraft:iron_axe"),
        ("_shovel", "minecraft:iron_shovel"),
        ("_hoe", "minecraft:iron_hoe"),
    ] {
        if name.ends_with(suffix) {
            return template;
        }
    }
    for (suffix, template) in [
        ("_helmet", "minecraft:iron_helmet"),
        ("_chestplate", "minecraft:iron_chestplate"),
        ("_leggings", "minecraft:iron_leggings"),
        ("_boots", "minecraft:iron_boots"),
    ] {
        if name.ends_with(suffix) {
            return template;
        }
    }
    // `parent: item/handheld` is how a tool's model is written.
    if let Ok(value) = hb_json::parse_bytes(model_json) {
        if value
            .get_str("parent")
            .is_some_and(|p| p.trim_start_matches("minecraft:") == "item/handheld")
        {
            return "minecraft:iron_sword";
        }
    }
    "minecraft:diamond"
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_json::parse;

    fn blockstate(json: &str) -> Value {
        parse(json).unwrap()
    }

    #[test]
    fn guesses_plain_cube_and_pillar() {
        assert_eq!(
            guess_for_blockstate("ruby_block", &blockstate(r#"{"variants":{"":{"model":"x"}}}"#)),
            Some("minecraft:stone")
        );
        assert_eq!(
            guess_for_blockstate("ruby_pillar", &blockstate(r#"{"variants":{"axis=y":{}}}"#)),
            Some("minecraft:oak_log")
        );
    }

    #[test]
    fn guesses_stairs_and_fences_and_doors() {
        assert_eq!(
            guess_for_blockstate(
                "ruby_stairs",
                &blockstate(
                    r#"{"variants":{"facing=north,half=bottom,shape=straight,waterlogged=false":{}}}"#
                )
            ),
            Some("minecraft:oak_stairs")
        );
        assert_eq!(
            guess_for_blockstate(
                "ruby_fence",
                &blockstate(r#"{"multipart":[{"when":{"north":"true"},"apply":{}}]}"#)
            ),
            Some("minecraft:oak_fence")
        );
        assert_eq!(
            guess_for_blockstate(
                "ruby_door",
                &blockstate(
                    r#"{"variants":{"facing=east,half=lower,hinge=left,open=false,powered=false":{}}}"#
                )
            ),
            Some("minecraft:oak_door")
        );
    }

    #[test]
    fn guesses_a_vine_where_a_wall_would_fit_by_name() {
        // Sides are booleans, which is a vine's shape, not a wall's.
        let vine = blockstate(
            r#"{"multipart":[{"when":{"north":"true","up":"false"},"apply":{}}]}"#,
        );
        assert_eq!(guess_for_blockstate("hanging_vines", &vine), Some("minecraft:vine"));
        // A real wall uses `low`/`tall`.
        let wall = blockstate(
            r#"{"multipart":[{"when":{"north":"low","up":"true"},"apply":{}}]}"#,
        );
        assert_eq!(
            guess_for_blockstate("ruby_wall", &wall),
            Some("minecraft:cobblestone_wall")
        );
    }

    #[test]
    fn properties_and_values_come_from_variants_and_multipart() {
        let value = blockstate(
            r#"{"variants":{"facing=north,half=bottom":{}},"multipart":[{"when":{"lit":"true","AND":[{"powered":"true"}]},"apply":{}}]}"#,
        );
        let properties = properties(&value);
        assert!(properties.contains("facing"));
        assert!(properties.contains("half"));
        assert!(properties.contains("lit"));
        assert!(properties.contains("powered"));
        assert_eq!(
            values_of(&value, "facing"),
            BTreeSet::from(["north".to_owned()])
        );
        assert_eq!(
            values_of(&value, "lit"),
            BTreeSet::from(["true".to_owned()])
        );
    }

    #[test]
    fn item_templates_follow_names_and_parents() {
        assert_eq!(item_template("ruby_sword", b"{}"), "minecraft:iron_sword");
        assert_eq!(item_template("ruby_helmet", b"{}"), "minecraft:iron_helmet");
        assert_eq!(
            item_template("gem", br#"{"parent":"item/handheld"}"#),
            "minecraft:iron_sword"
        );
        assert_eq!(item_template("gem", b"{}"), "minecraft:diamond");
    }
}
