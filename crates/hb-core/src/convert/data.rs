//! Data-file conversions: recipes, loot tables, advancements and tags.
//!
//! Rules and their sources:
//!
//! * **1.21 ingredient shape.** Before 1.20.5 an ingredient was
//!   `{"item": "..."}` or `{"tag": "..."}`; 26.x reads a bare id string or
//!   `{"tag": ...}` with `"#"`. Both forms appear in real 1.21 jars, so both
//!   are accepted and converted.
//! * **Recipe result.** Before 1.20.5 the result named its item as `item`;
//!   from 1.20.5 it is `id`. A 26.x game skips a recipe whose result has no
//!   id, silently: the first recorded case is a recipe that simply never
//!   appears.
//! * **Loot tables.** 26.x reads `condition` (one) where 1.21 read
//!   `conditions` (a list, `all_of` for several), and `modifier` where it read
//!   `functions`. Conditions and functions name their kind in `type`, not in a
//!   repeated key. A `{min, max}` number provider with no `type` was a uniform
//!   one in 1.21 and is refused in 26.x.
//! * **Entity predicates.** 26.x keys sub-predicates by registry name
//!   (`"minecraft:flags"`, `"minecraft:entity_type"`), which Horizon's
//!   `carry.rs` documents with the vanilla data that uses them.
//! * **Advancements.** An `enter_block` criterion names blocks in `blocks`;
//!   the old singular `block` is ignored and the criterion then matches any
//!   block.
//! * **Tags.** `"replace": true` drops earlier entries; a tag file whose
//!   `values` are objects (`{"id": ..., "required": false}`) still parses on
//!   1.21 and later, so only the `replace` and empty-tag cases are handled
//!   here.

use super::{Note, Outcome, Severity};
use hb_json::{patch::Editor, Object, Value};
use hb_versions::{Layout, Release};

/// The namespaced kinds a value may be, as `(key, new kind)` renames.
fn kind_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(|s| s.to_owned())
}

/// `{"<key>": kind, ...}` as `{"type": kind, ...}`, keeping the rest.
fn rename_kind(value: &mut Value, key: &str) -> bool {
    let Some(object) = value.as_obj_mut() else {
        return false;
    };
    if object.contains_key("type") {
        return false;
    }
    let Some(kind) = object.remove(key) else {
        return false;
    };
    object.insert_last("type", kind);
    true
}

/// `partial`: whether a change happened.
#[derive(Default)]
struct Tally {
    changes: usize,
}

impl Tally {
    fn mark(&mut self) -> bool {
        self.changes += 1;
        true
    }
}

/// Whether the target reads 1.20.5+'s recipe shapes (bare id ingredients and
/// `id` in a result).
fn modern_recipes(target: &Release) -> bool {
    use hb_versions::version_key;
    version_key(target.id) >= version_key("1.20.5")
}

pub fn recipe(
    value: &mut Value,
    editor: &mut Editor,
    _source: Layout,
    target: &'static Release,
    outcome: &mut Outcome,
    file: &str,
) {
    if !modern_recipes(target) {
        return;
    }
    // A non-vanilla recipe type comes from a loader's own serializer; the game
    // has no such type, so it is reported rather than mangled.
    let kind = kind_of(value, "type").unwrap_or_default();
    if !kind.starts_with("minecraft:") && !kind.is_empty() {
        outcome.note(
            Note::error(
                "data.recipe.custom_type",
                format!(
                    "recipe type `{kind}` is not a vanilla type: it needs a serializer from a \
                     loader, so this recipe is not carried"
                ),
            )
            .at(file.to_owned()),
        );
        return;
    }
    if value.has("neoforge:conditions") || value.has("forge:conditions") {
        outcome.note(
            Note::warn(
                "data.recipe.loader_conditions",
                "the recipe carries loader conditions (`neoforge:conditions`), which no vanilla \
                 game evaluates; it is carried without them",
            )
            .at(file.to_owned()),
        );
        if let Some(object) = value.as_obj_mut() {
            object.remove("neoforge:conditions");
            object.remove("forge:conditions");
        }
    }
    let mut tally = Tally::default();
    // Ingredients: `key` (shaped), `ingredients` (shapeless), and the fields
    // single-ingredient types use.
    let mut fields: Vec<String> = Vec::new();
    for field in ["key", "ingredients", "ingredient", "base", "addition", "template"] {
        if value.has(field) {
            fields.push(field.to_owned());
        }
    }
    for field in fields {
        let Some(node) = value.get_mut(&field) else {
            continue;
        };
        let mut changed = false;
        match field.as_str() {
            "key" => {
                if let Some(object) = node.as_obj_mut() {
                    for (slot, ingredient) in object.iter_mut() {
                        let path = format!("key/{}", hb_json::patch::join(&[slot]));
                        let before = ingredient.clone();
                        if normalize_ingredient(ingredient) {
                            changed = true;
                            editor.record(
                                "data.recipe.ingredient.object_to_string",
                                &path,
                                hb_json::patch::Node::Present(before),
                                hb_json::patch::Node::of(ingredient),
                                "1.20.5+ reads an ingredient as a bare id or `#tag`",
                            );
                        }
                    }
                }
            }
            "ingredients" => {
                if let Some(items) = node.as_arr_mut() {
                    for (i, ingredient) in items.iter_mut().enumerate() {
                        let before = ingredient.clone();
                        if normalize_ingredient(ingredient) {
                            changed = true;
                            editor.record(
                                "data.recipe.ingredient.object_to_string",
                                &format!("ingredients/{i}"),
                                hb_json::patch::Node::Present(before),
                                hb_json::patch::Node::of(ingredient),
                                "1.20.5+ reads an ingredient as a bare id or `#tag`",
                            );
                        }
                    }
                }
            }
            _ => {
                let before = node.clone();
                if normalize_ingredient(node) {
                    changed = true;
                    editor.record(
                        "data.recipe.ingredient.object_to_string",
                        &field,
                        hb_json::patch::Node::Present(before),
                        hb_json::patch::Node::of(node),
                        "1.20.5+ reads an ingredient as a bare id or `#tag`",
                    );
                }
            }
        }
        if changed {
            tally.mark();
        }
    }
    // Result: `item` -> `id` (plus `count`, which both eras read).
    if let Some(result) = value.get_mut("result") {
        if let Some(object) = result.as_obj_mut() {
            if object.get("id").is_none() {
                if let Some(item) = object.get("item").cloned() {
                    object.remove("item");
                    object.insert_last("id", item);
                    tally.mark();
                    editor.record(
                        "data.recipe.result.item_to_id",
                        "result",
                        hb_json::patch::Node::Absent,
                        hb_json::patch::Node::of(result),
                        "1.20.5+ reads the result's item as `id`; with `item` the game skips the \
                         recipe and it never appears",
                    );
                }
            }
        }
        // A result that is a bare string is a 1.20.5+ shorthand; leave it.
    }
    // Shapeless recipes before 1.20.5 wrote `ingredients` with `item` objects,
    // handled above. `group` and `category` are read in both eras.
    if tally.changes > 0 {
        outcome.note(
            Note::info(
                "data.recipe.converted",
                format!("{} 1.20.5+ change(s) made for {}", tally.changes, target.id),
            )
            .at(file.to_owned()),
        );
    }
}

/// Turns `{"item": id}` into `id` and `{"tag": t}` into `#t`; a list of
/// `{"item": ...}` becomes a list of ids. Returns whether anything changed.
pub fn normalize_ingredient(value: &mut Value) -> bool {
    match value {
        Value::Str(_) => false,
        Value::Obj(object) => {
            if object.len() != 1 {
                return false;
            }
            if let Some(Value::Str(id)) = object.get("item") {
                let id = id.clone();
                *value = Value::Str(id);
                return true;
            }
            if let Some(Value::Str(tag)) = object.get("tag") {
                let tag = format!("#{tag}");
                *value = Value::Str(tag);
                return true;
            }
            false
        }
        Value::Arr(items) => {
            // A 1.20.5+ list of alternatives is already bare ids; a 1.20.4 one
            // is a list of one-key objects.
            let mut changed = false;
            let all_single_item = !items.is_empty()
                && items.iter().all(|i| {
                    i.as_obj()
                        .is_some_and(|o| o.len() == 1 && o.get("item").is_some())
                });
            if all_single_item {
                for item in items.iter_mut() {
                    if normalize_ingredient(item) {
                        changed = true;
                    }
                }
                // A one-element alternative list is just the ingredient.
                if items.len() == 1 {
                    let only = items.remove(0);
                    *value = only;
                }
                return changed;
            }
            for item in items.iter_mut() {
                changed |= normalize_ingredient(item);
            }
            changed
        }
        _ => false,
    }
}

/// Loot table and loot function conversions.
pub fn loot(
    value: &mut Value,
    editor: &mut Editor,
    _source: Layout,
    target: &'static Release,
    outcome: &mut Outcome,
    file: &str,
) {
    // The 26.x shape is what Horizon's own port rewrites to; 1.21.x keeps the
    // list form, so this only runs for 26.x targets.
    if target.layout != Layout::ModernHorizon {
        return;
    }
    let mut changes = 0;
    upgrade_loot(value, editor, "", &mut changes);
    if changes > 0 {
        outcome.note(
            Note::info(
                "data.loot.converted",
                format!("{changes} 26.x loot change(s) made"),
            )
            .at(file.to_owned()),
        );
    }
}

fn upgrade_loot(value: &mut Value, editor: &mut Editor, path: &str, changes: &mut usize) {
    match value {
        Value::Arr(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                upgrade_loot(item, editor, &format!("{path}/{i}"), changes);
            }
        }
        Value::Obj(object) => {
            // Pools' `rolls` and `bonus_rolls` are number providers.
            if let Some(pools) = object.get_mut("pools").and_then(Value::as_arr_mut) {
                for (i, pool) in pools.iter_mut().enumerate() {
                    for key in ["rolls", "bonus_rolls"] {
                        if let Some(node) = pool.get_mut(key) {
                            let at = format!("{path}/pools/{i}/{key}");
                            if type_number(node, editor, &at) {
                                *changes += 1;
                            }
                        }
                    }
                }
            }
            // `conditions` (list) -> `condition` (one, `all_of` for several).
            if let Some(Value::Arr(mut list)) = object.remove("conditions") {
                let at = if path.is_empty() {
                    "conditions".to_owned()
                } else {
                    format!("{path}/conditions")
                };
                for (i, condition) in list.iter_mut().enumerate() {
                    upgrade_condition(condition, editor, &format!("{at}/{i}"), changes);
                }
                let one = match <[Value; 1]>::try_from(list) {
                    Ok([only]) => only,
                    Err(list) => Value::obj([
                        ("type", Value::str("minecraft:all_of")),
                        ("terms", Value::Arr(list)),
                    ]),
                };
                object.insert_last("condition", one);
                editor.record(
                    "data.loot.conditions_to_condition",
                    &at,
                    hb_json::patch::Node::Absent,
                    hb_json::patch::Node::Present(object.get("condition").cloned().unwrap_or(Value::Null)),
                    "26.x reads one `condition`; several become `minecraft:all_of`",
                );
                *changes += 1;
            }
            // `functions` -> `modifier`, each function naming its kind in `type`.
            if let Some(Value::Arr(mut list)) = object.remove("functions") {
                let at = if path.is_empty() {
                    "functions".to_owned()
                } else {
                    format!("{path}/functions")
                };
                for (i, function) in list.iter_mut().enumerate() {
                    let child = format!("{at}/{i}");
                    rename_kind(function, "function");
                    upgrade_function(function, editor, &child, changes);
                }
                object.insert_last("modifier", Value::Arr(list));
                editor.record(
                    "data.loot.functions_to_modifier",
                    &at,
                    hb_json::patch::Node::Absent,
                    hb_json::patch::Node::Present(
                        object.get("modifier").cloned().unwrap_or(Value::Null),
                    ),
                    "26.x reads loot functions under `modifier`",
                );
                *changes += 1;
            }
            // Entries carry `functions` too, with the same rename.
            let keys: Vec<String> = object.keys().map(|k| k.to_owned()).collect();
            for key in keys {
                if key == "condition" || key == "modifier" {
                    continue;
                }
                let child = if path.is_empty() {
                    hb_json::patch::join(&[&key])
                } else {
                    format!("{path}/{}", hb_json::patch::join(&[&key]))
                };
                if let Some(node) = object.get_mut(&key) {
                    upgrade_loot(node, editor, &child, changes);
                }
            }
        }
        _ => {}
    }
}

/// A number provider: `{min, max}` with no `type` was a uniform one.
fn type_number(value: &mut Value, editor: &mut Editor, path: &str) -> bool {
    let Some(object) = value.as_obj_mut() else {
        return false;
    };
    let mut changed = false;
    if !object.contains_key("type") && object.contains_key("min") && object.contains_key("max") {
        object.insert_last("type", Value::str("minecraft:uniform"));
        editor.record(
            "data.loot.number_provider_typed",
            &hb_json::patch::child(path, "type"),
            hb_json::patch::Node::Absent,
            hb_json::patch::Node::Present(Value::str("minecraft:uniform")),
            "1.21 read a bare {min,max} as uniform; 26.x dispatches on `type` and refuses it",
        );
        changed = true;
    }
    let kind = value.get_str("type").unwrap_or_default().to_owned();
    let bounds: &[&str] = match kind.strip_prefix("minecraft:").unwrap_or(&kind) {
        "uniform" => &["min", "max"],
        "binomial" => &["n", "p"],
        _ => &[],
    };
    for key in bounds {
        if let Some(bound) = value.get_mut(key) {
            let child = format!("{path}/{}", hb_json::patch::join(&[key]));
            changed |= type_number(bound, editor, &child);
        }
    }
    changed
}

/// Number providers inside one loot function.
fn upgrade_function(function: &mut Value, editor: &mut Editor, path: &str, changes: &mut usize) {
    let kind = function.get_str("type").unwrap_or_default().to_owned();
    let kind = kind.strip_prefix("minecraft:").unwrap_or(&kind).to_owned();
    let mut each = |list: &str, key: &str, function: &mut Value, changes: &mut usize| {
        let mut count = 0;
        if let Some(items) = function.get_mut(list).and_then(Value::as_arr_mut) {
            for (i, item) in items.iter_mut().enumerate() {
                if let Some(node) = item.get_mut(key) {
                    let at = format!("{path}/{list}/{i}/{key}");
                    if type_number(node, editor, &at) {
                        count += 1;
                    }
                }
            }
        }
        *changes += count;
    };
    match kind.as_str() {
        "set_stew_effect" => each("effects", "duration", function, changes),
        "set_attributes" => each("modifiers", "amount", function, changes),
        "set_enchantments" => {
            let mut count = 0;
            if let Some(levels) = function
                .get_mut("enchantments")
                .and_then(Value::as_obj_mut)
            {
                let keys: Vec<String> = levels.keys().map(|k| k.to_owned()).collect();
                for key in keys {
                    if let Some(node) = levels.get_mut(&key) {
                        let at = format!("{path}/enchantments/{}", hb_json::patch::join(&[&key]));
                        if type_number(node, editor, &at) {
                            count += 1;
                        }
                    }
                }
            }
            *changes += count;
        }
        "limit_count" => {
            let mut count = 0;
            if let Some(limit) = function.get_mut("limit") {
                for key in ["min", "max"] {
                    if let Some(node) = limit.get_mut(key) {
                        let at = format!("{path}/limit/{key}");
                        if type_number(node, editor, &at) {
                            count += 1;
                        }
                    }
                }
            }
            *changes += count;
        }
        other => {
            let key = match other {
                "set_count" | "enchanted_count_increase" => "count",
                "set_damage" => "damage",
                "enchant_with_levels" => "levels",
                "set_ominous_bottle_amplifier" => "amplifier",
                _ => return,
            };
            let mut count = 0;
            if let Some(node) = function.get_mut(key) {
                let at = format!("{path}/{key}");
                if type_number(node, editor, &at) {
                    count += 1;
                }
            }
            *changes += count;
        }
    }
    upgrade_loot(function, editor, path, changes);
}

/// One loot condition in 26.x's shape.
fn upgrade_condition(condition: &mut Value, editor: &mut Editor, path: &str, changes: &mut usize) {
    if rename_kind(condition, "condition") {
        *changes += 1;
        editor.record(
            "data.loot.condition_typed",
            path,
            hb_json::patch::Node::Absent,
            hb_json::patch::Node::of(condition),
            "26.x reads a condition's kind in `type`",
        );
    }
    if let Some(term) = condition.get_mut("term") {
        upgrade_condition(term, editor, &format!("{path}/term"), changes);
    }
    let terms_len = condition
        .get("terms")
        .and_then(Value::as_arr)
        .map(|t| t.len())
        .unwrap_or(0);
    for i in 0..terms_len {
        if let Some(term) = condition
            .get_mut("terms")
            .and_then(Value::as_arr_mut)
            .and_then(|t| t.get_mut(i))
        {
            upgrade_condition(term, editor, &format!("{path}/terms/{i}"), changes);
        }
    }
    // A `match_tool` restricted to shears or to silk touch is a named tool
    // condition in 26.x.
    if condition.get_str("type") == Some("minecraft:match_tool") {
        if let Some(named) = named_tool_condition(condition.get("predicate")) {
            let before = condition.clone();
            *condition = Value::obj([("type", Value::str(named))]);
            *changes += 1;
            editor.record(
                "data.loot.named_tool_condition",
                path,
                hb_json::patch::Node::Present(before),
                hb_json::patch::Node::of(condition),
                "26.x has named tool conditions for these predicates",
            );
            return;
        }
    }
    match condition.get_str("type") {
        Some("minecraft:entity_properties") => {
            if let Some(predicate) = condition.get_mut("predicate") {
                entity_predicate(predicate, editor, &format!("{path}/predicate"), changes);
            }
        }
        Some("minecraft:damage_source_properties") => {
            let mut count = 0;
            for key in ["direct_entity", "source_entity"] {
                if let Some(predicate) = condition
                    .get_mut("predicate")
                    .and_then(|p| p.get_mut(key))
                {
                    let at = format!("{path}/predicate/{key}");
                    entity_predicate(predicate, editor, &at, &mut count);
                }
            }
            *changes += count;
        }
        _ => {}
    }
}

/// 1.21 entity predicate fields that 26.x keys by registry name.
const ENTITY_FIELDS: [&str; 15] = [
    "distance",
    "movement",
    "location",
    "stepping_on",
    "movement_affected_by",
    "effects",
    "nbt",
    "flags",
    "equipment",
    "periodic_tick",
    "team",
    "slots",
    "components",
    "predicates",
    "type_specific",
];

/// Variant kinds whose `variant` 26.x matches as a data component.
const VARIANT_KINDS: [&str; 14] = [
    "axolotl", "cat", "chicken", "cow", "frog", "horse", "llama", "mooshroom", "painting",
    "parrot", "rabbit", "salmon", "tropical_fish", "villager",
];

/// Rewrites a 1.21 entity predicate into 26.x's keyed form.
fn entity_predicate(value: &mut Value, editor: &mut Editor, path: &str, changes: &mut usize) {
    let Some(object) = value.as_obj_mut() else {
        return;
    };
    // Captured once: some patches record the whole node, which needs an
    // immutable view while the object is borrowed mutably.
    let snapshot = Value::Obj(object.clone());
    // `type` was the entity type; 26.x spells it `minecraft:entity_type`.
    if let Some(entity_type) = object.remove("type") {
        object.insert_last("minecraft:entity_type", entity_type);
        *changes += 1;
        editor.record(
            "data.loot.entity_predicate_key",
            path,
            hb_json::patch::Node::Absent,
            hb_json::patch::Node::of(&snapshot),
            "26.x keys an entity predicate's type as `minecraft:entity_type`",
        );
    }
    for field in ENTITY_FIELDS {
        if let Some(field_value) = object.remove(field) {
            if field == "type_specific" {
                object.insert_last(format!("minecraft:{field}"), field_value);
            } else {
                object.insert_last(format!("minecraft:{field}"), field_value);
            }
            *changes += 1;
            editor.record(
                "data.loot.entity_predicate_key",
                &format!("{path}/{}", hb_json::patch::join(&[field])),
                hb_json::patch::Node::Absent,
                hb_json::patch::Node::of(&snapshot),
                "26.x keys entity predicate parts by registry name",
            );
        }
    }
    // `type_specific` with a kind of its own: `minecraft:type_specific/<kind>`.
    if let Some(type_specific) = object.remove("minecraft:type_specific") {
        let mut type_specific = type_specific;
        upgrade_type_specific(&mut type_specific, editor, path, changes);
        let kind = type_specific
            .get_str("type")
            .unwrap_or("any")
            .to_owned();
        let kind = kind.trim_start_matches("minecraft:").to_owned();
        let kind = if kind == "slime" { "cube_mob".to_owned() } else { kind };
        if let Some(ts) = type_specific.as_obj_mut() {
            ts.remove("type");
            let (variant, color) = (ts.remove("variant"), ts.remove("color"));
            let key = if kind == "sheep" {
                color.map(|c| ("sheep/color".to_owned(), c))
            } else if VARIANT_KINDS.contains(&kind.as_str()) {
                variant.map(|v| (format!("{kind}/variant"), v))
            } else {
                None
            };
            if let Some((name, value)) = key {
                if object.get("minecraft:components").is_none() {
                    object.insert_last("minecraft:components", Value::Obj(Object::new()));
                }
                if let Some(components) = object.get_mut("minecraft:components").and_then(Value::as_obj_mut) {
                    components.insert(format!("minecraft:{name}"), value);
                    *changes += 1;
                }
            }
            if !ts.is_empty() || !(kind == "any" || VARIANT_KINDS.contains(&kind.as_str())) {
                object.insert_last(
                    format!("minecraft:type_specific/{kind}"),
                    Value::Obj(ts.clone()),
                );
            }
        }
        *changes += 1;
    }
    // Nested predicates.
    for field in ["vehicle", "passenger", "targeted_entity"] {
        let keyed = format!("minecraft:{field}");
        let nested = if object.contains_key(&keyed) {
            object.get_mut(&keyed)
        } else {
            object.get_mut(field)
        };
        if let Some(nested) = nested {
            entity_predicate(nested, editor, &format!("{path}/{field}"), changes);
        }
    }
}

fn upgrade_type_specific(value: &mut Value, editor: &mut Editor, path: &str, changes: &mut usize) {
    for field in ["looking_at", "entity_struck"] {
        if let Some(nested) = value.get_mut(field) {
            entity_predicate(nested, editor, &format!("{path}/{field}"), changes);
        }
    }
}

/// 26.x's named tool conditions for the 1.21 predicates they replace.
fn named_tool_condition(predicate: Option<&Value>) -> Option<&'static str> {
    let predicate = predicate?;
    let keys: Vec<&str> = predicate.as_obj()?.keys().collect();
    match keys.as_slice() {
        ["predicates"] => {
            let inner = predicate.get("predicates")?;
            if inner.as_obj()?.len() != 1 {
                return None;
            }
            let list = inner.get("minecraft:enchantments")?.as_arr()?;
            let [only] = list else {
                return None;
            };
            let silk = only.get_str("enchantments") == Some("minecraft:silk_touch")
                && only
                    .get("levels")
                    .and_then(|l| l.get("min"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1)
                    <= 1
                && only.get("levels").and_then(|l| l.get("max")).is_none();
            silk.then_some("minecraft:tool/can_silk_touch")
        }
        ["items"] => {
            let items = predicate.get("items")?;
            let shears = items.as_str() == Some("minecraft:shears")
                || items
                    .as_arr()
                    .is_some_and(|a| a.len() == 1 && a[0].as_str() == Some("minecraft:shears"));
            shears.then_some("minecraft:tool/can_shear")
        }
        _ => None,
    }
}

/// Advancements: an `enter_block` criterion's `block` is `blocks` in 26.x.
pub fn advancement(
    value: &mut Value,
    editor: &mut Editor,
    _source: Layout,
    target: &'static Release,
    outcome: &mut Outcome,
    file: &str,
) {
    if target.layout != Layout::ModernHorizon {
        return;
    }
    let mut changes = 0;
    if let Some(criteria) = value.get_mut("criteria").and_then(Value::as_obj_mut) {
        let names: Vec<String> = criteria.keys().map(|k| k.to_owned()).collect();
        for name in names {
            let Some(criterion) = criteria.get_mut(&name) else {
                continue;
            };
            let trigger = criterion.get_str("trigger").unwrap_or_default().to_owned();
            if trigger != "minecraft:enter_block" && trigger != "enter_block" {
                continue;
            }
            let at = format!("criteria/{}/conditions", hb_json::patch::join(&[&name]));
            if let Some(conditions) = criterion.get_mut("conditions").and_then(Value::as_obj_mut) {
                if !conditions.contains_key("blocks") {
                    if let Some(block) = conditions.remove("block") {
                        conditions.insert_last("blocks", block);
                        changes += 1;
                        editor.record(
                            "data.advancement.block_to_blocks",
                            &at,
                            hb_json::patch::Node::Absent,
                            hb_json::patch::Node::Present(Value::Obj(conditions.clone())),
                            "26.x reads a block list in `blocks`; the old `block` matched any block",
                        );
                    }
                }
            }
        }
    }
    if changes > 0 {
        outcome.note(
            Note::info(
                "data.advancement.converted",
                format!("{changes} enter_block criterion(s) rewritten for 26.x"),
            )
            .at(file.to_owned()),
        );
    }
}

/// Tags: report `replace: true`, which drops everything the pack before it
/// added, and an empty tag, which vanilla treats as "no entries" — both are
/// legal but are the two ways a tag silently loses content.
pub fn tag(
    value: &mut Value,
    _editor: &mut Editor,
    target: &Release,
    outcome: &mut Outcome,
    file: &str,
) {
    let Some(object) = value.as_obj() else {
        outcome.note(
            Note::error(
                "data.tag.not_an_object",
                "a tag file must be a JSON object with `values`",
            )
            .at(file.to_owned()),
        );
        return;
    };
    let replace = object.get("replace").and_then(Value::as_bool).unwrap_or(false);
    if replace {
        outcome.note(
            Note::warn(
                "data.tag.replace",
                "`\"replace\": true` drops the entries earlier packs added; the log says so when it \
                 loads, and the tag will contain only this file's entries",
            )
            .at(file.to_owned()),
        );
    }
    match object.get("values") {
        Some(Value::Arr(items)) if items.is_empty() => {
            outcome.note(
                Note::warn(
                    "data.tag.empty",
                    "an empty tag: legal, but anything matching it will match nothing",
                )
                .at(file.to_owned()),
            );
        }
        Some(Value::Arr(_)) => {}
        Some(_) => {
            outcome.note(
                Note::error(
                    "data.tag.values_type",
                    "`values` must be an array of ids, `#tags` or {id, required} objects",
                )
                .at(file.to_owned()),
            );
        }
        None => {
            outcome.note(
                Note::error(
                    "data.tag.no_values",
                    "a tag file with no `values` loads as an empty tag",
                )
                .at(file.to_owned()),
            );
        }
    }
    if target.layout == Layout::Legacy {
        outcome.note(
            Note::info(
                "data.tag.legacy",
                "1.8.9 reads no JSON tags; its equivalent is the code's `oreDictionary` or \
                 `GameRegistry` entries",
            )
            .at(file.to_owned()),
        );
    }
    let _ = Severity::Info;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_json::parse;

    fn target(id: &str) -> &'static Release {
        hb_versions::release(id).unwrap()
    }

    fn empty_outcome() -> Outcome {
        Outcome::default()
    }

    #[test]
    fn normalizes_both_ingredient_shapes() {
        let mut v = parse(r#"{"item":"minecraft:iron_ingot"}"#).unwrap();
        assert!(normalize_ingredient(&mut v));
        assert_eq!(hb_json::to_string(&v), r#""minecraft:iron_ingot""#);

        let mut v = parse(r#"{"tag":"c:ingots"}"#).unwrap();
        assert!(normalize_ingredient(&mut v));
        assert_eq!(hb_json::to_string(&v), "\"#c:ingots\"");

        // Already modern.
        let mut v = parse(r#""minecraft:stick""#).unwrap();
        assert!(!normalize_ingredient(&mut v));

        // A one-element alternative list collapses.
        let mut v = parse(r#"[{"item":"minecraft:stick"}]"#).unwrap();
        assert!(normalize_ingredient(&mut v));
        assert_eq!(hb_json::to_string(&v), r#""minecraft:stick""#);

        // A multi-element one becomes bare ids.
        let mut v = parse(r#"[{"item":"a:b"},{"item":"c:d"}]"#).unwrap();
        assert!(normalize_ingredient(&mut v));
        assert_eq!(hb_json::to_string(&v), r#"["a:b","c:d"]"#);
    }

    #[test]
    fn converts_a_1_21_shaped_recipe() {
        // A shaped recipe's key is `#`, so the JSON is built here rather than
        // written as a raw literal whose delimiter would collide.
        let source = [
            "{\"type\":\"minecraft:crafting_shaped\",\"pattern\":[\"##\",\"##\"],",
            "\"key\":{\"#\":{\"item\":\"minecraft:iron_ingot\"}},",
            "\"result\":{\"item\":\"example:block\",\"count\":1}}",
        ]
        .concat();
        let mut v = parse(&source).unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        recipe(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "data/example/recipe/block.json",
        );
        assert_eq!(
            v.get("key").unwrap().get("#").unwrap().as_str(),
            Some("minecraft:iron_ingot")
        );
        assert_eq!(v.get("result").unwrap().get_str("id"), Some("example:block"));
        assert!(v.get("result").unwrap().get("item").is_none());
        assert_eq!(v.get("result").unwrap().get("count").unwrap().as_i64(), Some(1));
        let codes: Vec<&str> = editor.patches().iter().map(|p| p.code).collect();
        assert!(codes.contains(&"data.recipe.ingredient.object_to_string"));
        assert!(codes.contains(&"data.recipe.result.item_to_id"));
    }

    #[test]
    fn refuses_a_custom_recipe_type_with_a_reason() {
        let mut v = parse(r#"{"type":"create:mechanical_crafting","result":{"item":"a:b"}}"#).unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        recipe(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "data/x/recipe/y.json",
        );
        let unsupported: Vec<&Note> = outcome.unsupported().collect();
        assert_eq!(unsupported.len(), 1);
        assert!(unsupported[0].message.contains("create:mechanical_crafting"));
        assert_eq!(unsupported[0].code, "data.recipe.custom_type");
    }

    #[test]
    fn converts_loot_lists_to_26_x_singular_keys() {
        let mut v = parse(
            r#"{"pools":[{"rolls":{"min":1,"max":3},"entries":[{"type":"minecraft:item",
                "name":"example:gem",
                "functions":[{"function":"minecraft:set_count","count":{"min":1,"max":2}}],
                "conditions":[{"condition":"minecraft:survives_explosion"}]}]}]}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        loot(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "data/example/loot_table/x.json",
        );
        let pool = &v.get("pools").unwrap().as_arr().unwrap()[0];
        assert_eq!(
            pool.get("rolls").unwrap().get_str("type"),
            Some("minecraft:uniform")
        );
        let entry = &pool.get("entries").unwrap().as_arr().unwrap()[0];
        let condition = entry.get("condition").unwrap();
        assert_eq!(
            condition.get_str("type"),
            Some("minecraft:survives_explosion")
        );
        assert!(entry.get("conditions").is_none());
        let modifier = entry.get("modifier").unwrap().as_arr().unwrap();
        assert_eq!(modifier[0].get_str("type"), Some("minecraft:set_count"));
        assert_eq!(
            modifier[0].get("count").unwrap().get_str("type"),
            Some("minecraft:uniform")
        );
        assert!(modifier[0].get("function").is_none());
    }

    #[test]
    fn several_conditions_become_all_of() {
        let mut v = parse(
            r#"{"pools":[{"conditions":[{"condition":"minecraft:survives_explosion"},
                {"condition":"minecraft:random_chance","chance":0.5}]}]}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        loot(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "f.json",
        );
        let condition = &v.get("pools").unwrap().as_arr().unwrap()[0]
            .get("condition")
            .unwrap()
            .clone();
        assert_eq!(condition.get_str("type"), Some("minecraft:all_of"));
        let terms = condition.get("terms").unwrap().as_arr().unwrap();
        assert_eq!(terms.len(), 2);
        assert_eq!(
            terms[0].get_str("type"),
            Some("minecraft:survives_explosion")
        );
    }

    #[test]
    fn converts_named_tool_conditions() {
        for (predicate, expected) in [
            (
                r#"{"items":"minecraft:shears"}"#,
                "minecraft:tool/can_shear",
            ),
            (
                r#"{"predicates":{"minecraft:enchantments":[{"enchantments":"minecraft:silk_touch","levels":{"min":1}}]}}"#,
                "minecraft:tool/can_silk_touch",
            ),
        ] {
            let mut v = parse(&format!(
                r#"{{"pools":[{{"entries":[{{"type":"minecraft:item","name":"a:b",
                    "conditions":[{{"condition":"minecraft:match_tool","predicate":{predicate}}}]}}]}}]}}"#
            ))
            .unwrap();
            let mut editor = Editor::new();
            let mut outcome = empty_outcome();
            loot(
                &mut v,
                &mut editor,
                Layout::SingularFolders,
                target("26.3"),
                &mut outcome,
                "f.json",
            );
            let condition = v.get("pools").unwrap().as_arr().unwrap()[0]
                .get("entries").unwrap().as_arr().unwrap()[0]
                .get("condition")
                .unwrap()
                .clone();
            assert_eq!(condition.get_str("type"), Some(expected));
            assert!(condition.get("predicate").is_none(), "predicate is gone");
        }
    }

    #[test]
    fn converts_entity_predicates() {
        let mut v = parse(
            r#"{"pools":[{"entries":[{"type":"minecraft:item","name":"a:b",
              "conditions":[{"condition":"minecraft:entity_properties","entity":"this",
                "predicate":{"type":"minecraft:cow","flags":{"is_on_fire":true}}}]}]}]}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        loot(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "f.json",
        );
        let predicate = &v.get("pools").unwrap().as_arr().unwrap()[0]
            .get("entries").unwrap().as_arr().unwrap()[0]
            .get("condition").unwrap()
            .get("predicate").unwrap()
            .clone();
        assert_eq!(
            predicate.get_str("minecraft:entity_type"),
            Some("minecraft:cow")
        );
        assert!(predicate
            .get("minecraft:flags")
            .unwrap()
            .get("is_on_fire")
            .unwrap()
            .as_bool()
            .unwrap());
        assert!(predicate.get("type").is_none());
    }

    #[test]
    fn variants_become_components() {
        let mut v = parse(
            r#"{"pools":[{"entries":[{"type":"minecraft:item","name":"a:b","conditions":[
                {"condition":"minecraft:entity_properties","entity":"this",
                 "predicate":{"type_specific":{"type":"minecraft:sheep","color":"black"}}}]}]}]}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        loot(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "f.json",
        );
        let predicate = &v.get("pools").unwrap().as_arr().unwrap()[0]
            .get("entries").unwrap().as_arr().unwrap()[0]
            .get("condition").unwrap()
            .get("predicate").unwrap()
            .clone();
        assert_eq!(
            predicate
                .get("minecraft:components")
                .unwrap()
                .get_str("minecraft:sheep/color"),
            Some("black"),
            "{}",
            hb_json::to_string(predicate)
        );
    }

    #[test]
    fn leaves_1_21_loot_alone_for_1_21_targets() {
        let original = r#"{"pools":[{"conditions":[{"condition":"minecraft:survives_explosion"}]}]}"#;
        let mut v = parse(original).unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        loot(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("1.21.1"),
            &mut outcome,
            "f.json",
        );
        assert!(editor.patches().is_empty());
        assert_eq!(hb_json::to_string(&v), original);
    }

    #[test]
    fn converts_enter_block_advancements() {
        let mut v = parse(
            r#"{"criteria":{"in":{"trigger":"minecraft:enter_block",
                "conditions":{"block":"minecraft:stone","state":{}}}}}"#,
        )
        .unwrap();
        let mut editor = Editor::new();
        let mut outcome = empty_outcome();
        advancement(
            &mut v,
            &mut editor,
            Layout::SingularFolders,
            target("26.3"),
            &mut outcome,
            "a.json",
        );
        let conditions = v.get("criteria").unwrap().get("in").unwrap().get("conditions").unwrap();
        assert_eq!(conditions.get_str("blocks"), Some("minecraft:stone"));
        assert!(conditions.get("block").is_none());
    }

    #[test]
    fn reports_replace_and_empty_tags() {
        let mut outcome = empty_outcome();
        let mut editor = Editor::new();
        let mut v = parse(r#"{"replace":true,"values":[]}"#).unwrap();
        tag(&mut v, &mut editor, target("26.3"), &mut outcome, "t.json");
        assert_eq!(outcome.warnings().count(), 2);
        let codes: Vec<&str> = outcome.notes.iter().map(|n| n.code).collect();
        assert!(codes.contains(&"data.tag.replace"));
        assert!(codes.contains(&"data.tag.empty"));
    }
}
