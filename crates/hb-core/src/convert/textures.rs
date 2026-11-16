//! Texture-related conversions beyond the PNG check.
//!
//! * 1.8.9 `textures/blocks/` and `textures/items/` became `textures/block/`
//!   and `textures/item/` in 1.13 (handled with paths, in `mod.rs`).
//! * 1.21.11 split item textures out of the blocks atlas into an `items`
//!   atlas; a texture used by both a block and an item model has to exist in
//!   both, which is a real 26.x requirement and worth telling the author
//!   about.
//! * 1.21.9 renamed `block/chain` to `block/iron_chain`, which `data.rs`
//!   covers for ids and models refer to through `minecraft:` names.

use super::Outcome;
use hb_versions::Release;
use std::collections::BTreeMap;

/// Reports textures that both a block model and an item model reference: in
/// 26.x they live in different atlases, so one file cannot serve both.
pub fn cross_atlas_references(
    block_textures: &std::collections::BTreeSet<String>,
    item_textures: &std::collections::BTreeSet<String>,
    target: &Release,
    outcome: &mut Outcome,
) {
    if target.layout != hb_versions::Layout::ModernHorizon {
        return;
    }
    // The split happened in 1.21.11's resource format 75.
    if target.resource_format.major < 75 {
        return;
    }
    let shared: Vec<&String> = block_textures
        .intersection(item_textures)
        .collect();
    if shared.is_empty() {
        return;
    }
    let mut sample: BTreeMap<&str, ()> = BTreeMap::new();
    for texture in &shared {
        sample.insert(texture.as_str(), ());
    }
    outcome.note(super::Note::warn(
        "asset.atlas.cross_reference",
        format!(
            "{} texture(s) are used by both block and item models ({}): since 1.21.11 these are \
             separate atlases, so each needs its own copy",
            shared.len(),
            shared
                .iter()
                .take(5)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn reports_shared_atlas_textures_only_for_26_x() {
        let mut block = BTreeSet::new();
        block.insert("example:block/gem".to_owned());
        let mut item = BTreeSet::new();
        item.insert("example:block/gem".to_owned());
        item.insert("example:item/gem".to_owned());

        let mut outcome = Outcome::default();
        cross_atlas_references(
            &block,
            &item,
            hb_versions::release("26.3").unwrap(),
            &mut outcome,
        );
        assert_eq!(outcome.warnings().count(), 1);

        let mut outcome = Outcome::default();
        cross_atlas_references(
            &block,
            &item,
            hb_versions::release("1.21.8").unwrap(),
            &mut outcome,
        );
        assert_eq!(outcome.warnings().count(), 0, "one atlas before 1.21.11");
    }
}
