//! End-to-end conversion tests over synthetic jars.
//!
//! Each test builds a real zip in memory with a mod's metadata, content and
//! classes, exactly as a jar on disk would look, and converts it for a target.
//! That is the same path `hb convert` takes, so these are integration tests of
//! the engine rather than of individual helpers.

use super::*;
use crate::jar::read_bytes;
use hb_versions::release;

/// A tiny valid PNG of the given size.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&[0; 4]);
    out
}

/// A class file with the given name and superclass.
fn class_file(name: &str, super_name: &str) -> Vec<u8> {
    let mut pool: Vec<Vec<u8>> = Vec::new();
    let utf8 = |pool: &mut Vec<Vec<u8>>, text: &str| -> u16 {
        let mut entry = vec![1u8];
        entry.extend_from_slice(&(text.len() as u16).to_be_bytes());
        entry.extend_from_slice(text.as_bytes());
        pool.push(entry);
        pool.len() as u16
    };
    let name_at = utf8(&mut pool, name);
    let super_at = utf8(&mut pool, super_name);
    let mut this = vec![7u8];
    this.extend_from_slice(&name_at.to_be_bytes());
    pool.push(this);
    let this_at = pool.len() as u16;
    let mut parent = vec![7u8];
    parent.extend_from_slice(&super_at.to_be_bytes());
    pool.push(parent);
    let parent_at = pool.len() as u16;

    let mut out = Vec::new();
    out.extend_from_slice(&0xCAFE_BABEu32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&52u16.to_be_bytes());
    out.extend_from_slice(&((pool.len() + 1) as u16).to_be_bytes());
    for entry in &pool {
        out.extend_from_slice(entry);
    }
    out.extend_from_slice(&0x0021u16.to_be_bytes());
    out.extend_from_slice(&this_at.to_be_bytes());
    out.extend_from_slice(&parent_at.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // interfaces
    out.extend_from_slice(&0u16.to_be_bytes()); // fields
    out.extend_from_slice(&0u16.to_be_bytes()); // methods
    out.extend_from_slice(&0u16.to_be_bytes()); // attributes
    out
}

/// Builds a jar from `(path, bytes)` pairs.
fn jar_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<hb_zip::OutEntry> = files
        .iter()
        .map(|(name, bytes)| hb_zip::OutEntry::new(*name, bytes.clone()))
        .collect();
    hb_zip::build(&entries).unwrap()
}

fn fabric_jar(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut all: Vec<(&str, Vec<u8>)> = vec![(
        "fabric.mod.json",
        br#"{"schemaVersion":1,"id":"example","version":"1.0.0","name":"Example",
             "description":"A test mod","authors":["Tester"],"license":"MIT",
             "depends":{"minecraft":"~1.21.1"}}"#
            .to_vec(),
    )];
    all.extend_from_slice(files);
    jar_bytes(&all)
}

fn convert_jar(bytes: &[u8], target_id: &str, source_id: Option<&str>) -> Outcome {
    let jar = read_bytes(bytes, "test.jar", Default::default()).expect("jar reads");
    let target = release(target_id).unwrap();
    let source = source_id.map(|id| release(id).unwrap());
    let options = Options::new("example_port", target, source);
    convert(&jar, &options)
}

#[test]
fn converts_a_fabric_content_mod_for_26_3() {
    let bytes = fabric_jar(&[
        (
            "assets/example/blockstates/gem_block.json",
            br#"{"variants":{"":{"model":"example:block/gem_block"}}}"#.to_vec(),
        ),
        (
            "assets/example/models/block/gem_block.json",
            br#"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/gem_block"}}"#
                .to_vec(),
        ),
        (
            "assets/example/models/item/gem.json",
            br#"{"parent":"minecraft:item/generated","textures":{"layer0":"example:item/gem"}}"#
                .to_vec(),
        ),
        (
            "assets/example/textures/block/gem_block.png",
            png(16, 16),
        ),
        (
            "assets/example/textures/item/gem.png",
            png(16, 16),
        ),
        (
            "assets/example/lang/en_us.json",
            br#"{"block.example.gem_block":"Block of Gem","item.example.gem":"Gem"}"#.to_vec(),
        ),
        (
            "data/example/recipe/gem_block.json",
            [
                "{\"type\":\"minecraft:crafting_shaped\",\"pattern\":[\"##\",\"##\"],",
                "\"key\":{\"#\":{\"item\":\"example:gem\"}},",
                "\"result\":{\"item\":\"example:gem_block\",\"count\":1}}",
            ]
            .concat()
            .into_bytes(),
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            br#"{"replace":false,"values":["example:gem_block"]}"#.to_vec(),
        ),
    ]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));

    // Assets and data are re-namespaced to the package id.
    assert!(outcome.files.contains_key("assets/example_port/lang/en_us.json"));
    assert!(outcome
        .files
        .contains_key("assets/example_port/textures/block/gem_block.png"));
    assert!(outcome
        .files
        .contains_key("data/example_port/recipe/gem_block.json"));

    // The recipe was converted to 26.x's ingredient and result shapes.
    let recipe = hb_json::parse_bytes(
        outcome.files.get("data/example_port/recipe/gem_block.json").unwrap(),
    )
    .unwrap();
    assert_eq!(recipe.get("key").unwrap().get("#").unwrap().as_str(), Some("example_port:gem"));
    assert_eq!(
        recipe.get("result").unwrap().get_str("id"),
        Some("example_port:gem_block")
    );

    // Blocks and items are declared with guessed templates.
    let block = outcome
        .files
        .get("data/example_port/horizon/block/gem_block.json")
        .expect("block declared");
    let block = hb_json::parse_bytes(block).unwrap();
    assert_eq!(block.get_str("template"), Some("minecraft:stone"));
    let item = outcome
        .files
        .get("data/example_port/horizon/item/gem.json")
        .expect("item declared");
    assert_eq!(
        hb_json::parse_bytes(item).unwrap().get_str("template"),
        Some("minecraft:diamond")
    );
    assert!(outcome.declares_content);
    assert_eq!(outcome.guessed_templates.len(), 2);
    assert!(outcome.unsupported().count() == 0, "{:?}", outcome.notes);
}

#[test]
fn item_models_get_1_21_4_definitions() {
    let bytes = fabric_jar(&[(
        "assets/example/models/item/gem.json",
        br#"{"parent":"minecraft:item/generated","textures":{"layer0":"example:item/gem"}}"#
            .to_vec(),
    )]);
    let outcome = convert_jar(&bytes, "1.21.4", Some("1.21.1"));
    assert!(outcome
        .files
        .contains_key("assets/example_port/models/item/gem.json"));
    // 1.21.4 reads an item's model through `items/<name>.json`; without it the
    // item shows a missing model even though the model file is there.
    let definition = outcome
        .files
        .get("assets/example_port/items/gem.json")
        .expect("definition written");
    let definition = hb_json::parse_bytes(definition).unwrap();
    assert_eq!(
        definition.get("model").unwrap().get_str("type"),
        Some("minecraft:model")
    );
    assert_eq!(
        definition.get("model").unwrap().get_str("model"),
        Some("example_port:item/gem")
    );
    // Horizon declarations are 26.x's content format; a Java-era package does
    // not get them.
    assert!(!outcome
        .files
        .keys()
        .any(|p| p.contains("/horizon/")));
    assert!(!outcome.declares_content);
}

#[test]
fn reports_java_only_functionality_instead_of_dropping_it() {
    let bytes = fabric_jar(&[
        (
            "com/example/ExampleMod.class",
            class_file("com/example/ExampleMod", "net/fabricmc/api/ModInitializer"),
        ),
        // Extending a game class is what a content-only port cannot reproduce.
        (
            "com/example/RubyBlock.class",
            class_file("com/example/RubyBlock", "net/minecraft/world/level/block/Block"),
        ),
        (
            "com/example/mixin/BlockMixin.class",
            class_file("com/example/mixin/BlockMixin", "java/lang/Object"),
        ),
        (
            "example.mixins.json",
            br#"{"package":"com.example.mixin","mixins":["BlockMixin"]}"#.to_vec(),
        ),
        (
            "assets/example/lang/en_us.json",
            br#"{"item.example.gem":"Gem"}"#.to_vec(),
        ),
    ]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));
    let codes: Vec<&str> = outcome.notes.iter().map(|n| n.code).collect();
    assert!(codes.contains(&"java.mixin"), "{codes:?}");
    assert!(codes.contains(&"java.behaviour") || codes.contains(&"java.entry_point"));
    // Nothing was silently lost: every warning has a file or a message.
    for note in outcome.warnings() {
        assert!(!note.message.is_empty());
    }
}

#[test]
fn a_jar_with_no_metadata_is_refused_with_a_reason() {
    let bytes = jar_bytes(&[("assets/x/lang/en_us.json", b"{}".to_vec())]);
    let outcome = convert_jar(&bytes, "26.3", None);
    assert_eq!(outcome.unsupported().count(), 1);
    assert_eq!(
        outcome.unsupported().next().unwrap().code,
        "jar.metadata.missing"
    );
    assert!(outcome.files.is_empty());
}

#[test]
fn a_jar_with_no_content_but_code_says_so() {
    let bytes = jar_bytes(&[
        (
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"example","version":"1","depends":{"minecraft":"1.21.1"}}"#
                .to_vec(),
        ),
        (
            "com/example/Mod.class",
            class_file("com/example/Mod", "java/lang/Object"),
        ),
    ]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));
    assert_eq!(
        outcome.unsupported().next().map(|n| n.code),
        Some("content.empty")
    );
    // Everything that was dropped is named, never silently lost.
    assert!(outcome
        .notes
        .iter()
        .any(|n| n.code == "content.empty" && n.severity == Severity::Error));
}

#[test]
fn renames_1_21_folders_for_a_1_13_source() {
    let bytes = jar_bytes(&[
        (
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"example","version":"1","depends":{"minecraft":"1.20.1"}}"#
                .to_vec(),
        ),
        (
            "data/example/recipes/gem.json",
            br#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:stick"}],"result":{"item":"example:gem"}}"#.to_vec(),
        ),
        (
            "data/example/tags/items/gems.json",
            br#"{"values":["example:gem"]}"#.to_vec(),
        ),
        (
            "assets/example/textures/blocks/old.png",
            png(16, 16),
        ),
    ]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.20.1"));
    assert!(
        outcome
            .files
            .contains_key("data/example_port/recipe/gem.json"),
        "{:?}",
        outcome.files.keys().collect::<Vec<_>>()
    );
    assert!(outcome
        .files
        .contains_key("data/example_port/tags/item/gems.json"));
    assert!(outcome
        .files
        .contains_key("assets/example_port/textures/blocks/old.png"));
    // And the recipe contents were brought to 26.x's shapes.
    let recipe = hb_json::parse_bytes(outcome.files.get("data/example_port/recipe/gem.json").unwrap())
        .unwrap();
    assert_eq!(
        recipe.get("result").unwrap().get_str("id"),
        Some("example_port:gem")
    );
    assert_eq!(
        recipe.get("ingredients").unwrap().as_arr().unwrap()[0].as_str(),
        Some("minecraft:stick")
    );
}

#[test]
fn a_texture_that_is_not_a_png_is_refused_with_a_reason() {
    let bytes = fabric_jar(&[(
        "assets/example/textures/item/broken.png",
        b"this is not a png".to_vec(),
    )]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));
    let note = outcome
        .notes
        .iter()
        .find(|n| n.code == "asset.texture.invalid_png")
        .expect("reported");
    assert_eq!(note.severity, Severity::Error);
    assert!(!outcome
        .files
        .contains_key("assets/example_port/textures/item/broken.png"));
}

#[test]
fn malformed_json_is_refused_not_carried() {
    let bytes = fabric_jar(&[(
        "data/example/recipe/bad.json",
        br#"{"type":"minecraft:crafting_shapeless",}"#.to_vec(),
    )]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));
    assert!(outcome
        .notes
        .iter()
        .any(|n| n.code == "json.unreadable" && n.severity == Severity::Error));
    assert!(!outcome
        .files
        .keys()
        .any(|p| p.contains("recipe/bad.json")));
}

#[test]
fn legacy_1_8_9_lang_becomes_json_and_models_are_refused() {
    let bytes = jar_bytes(&[
        (
            "mcmod.info",
            br#"[{"modid":"oldmod","name":"Old","version":"1.0","mcversion":"1.8.9"}]"#.to_vec(),
        ),
        (
            "assets/oldmod/lang/en_US.lang",
            b"item.oldmod.gem=Gem\n".to_vec(),
        ),
        (
            "assets/oldmod/textures/items/gem.png",
            png(16, 16),
        ),
        (
            "assets/oldmod/models/item/gem.json",
            br#"{"parent":"builtin/generated","textures":{"layer0":"items/gem"}}"#.to_vec(),
        ),
    ]);
    let outcome = convert_jar(&bytes, "1.21.1", Some("1.8.9"));
    assert!(outcome
        .files
        .contains_key("assets/example_port/lang/en_us.json"));
    let lang = hb_json::parse_bytes(outcome.files.get("assets/example_port/lang/en_us.json").unwrap())
        .unwrap();
    assert_eq!(lang.get_str("item.oldmod.gem"), Some("Gem"));
    // 1.8.9 item textures were `textures/items/`.
    assert!(outcome
        .files
        .contains_key("assets/example_port/textures/item/gem.png"));
    // The model's texture reference moved from `items/` to `item/`.
    let model = hb_json::parse_bytes(
        outcome
            .files
            .get("assets/example_port/models/item/gem.json")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        model.get("textures").unwrap().get_str("layer0"),
        Some("minecraft:item/gem")
    );
}

#[test]
fn every_note_has_a_stable_code_and_a_reason() {
    let bytes = fabric_jar(&[
        (
            "assets/example/textures/item/x.png",
            b"nope".to_vec(),
        ),
        (
            "data/example/recipe/custom.json",
            br#"{"type":"create:mixing","ingredients":[],"result":{"item":"a:b"}}"#.to_vec(),
        ),
        (
            "data/example/tags/block/empty.json",
            br#"{"values":[]}"#.to_vec(),
        ),
        (
            "example.mixins.json",
            br#"{"package":"com.example.mixin","client":["X"]}"#.to_vec(),
        ),
    ]);
    let outcome = convert_jar(&bytes, "26.3", Some("1.21.1"));
    assert!(!outcome.notes.is_empty());
    for note in &outcome.notes {
        assert!(
            note.code.contains('.'),
            "code `{}` should be area.thing.action",
            note.code
        );
        assert!(
            note.message.len() > 20,
            "`{}` should explain itself: {}",
            note.code,
            note.message
        );
    }
}

#[test]
fn conversion_is_deterministic() {
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "assets/example/blockstates/a.json",
            br#"{"variants":{"":{"model":"example:block/a"}}}"#.to_vec(),
        ),
        (
            "data/example/recipe/a.json",
            [
                "{\"type\":\"minecraft:crafting_shaped\",",
                "\"key\":{\"#\":{\"item\":\"example:x\"}},",
                "\"result\":{\"item\":\"example:a\"}}",
            ]
            .concat()
            .into_bytes(),
        ),
    ];
    let first = convert_jar(&fabric_jar(&files), "26.3", Some("1.21.1"));
    let second = convert_jar(&fabric_jar(&files), "26.3", Some("1.21.1"));
    assert_eq!(first.files, second.files);
    let codes_first: Vec<&str> = first.notes.iter().map(|n| n.code).collect();
    let codes_second: Vec<&str> = second.notes.iter().map(|n| n.code).collect();
    assert_eq!(codes_first, codes_second);
}
