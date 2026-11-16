//! Assembling a `.hmod`, and — for the versions where it is true — a plain
//! Java Edition resource/data pack.
//!
//! Two different products, deliberately kept apart:
//!
//! * A **Horizon package** (`.hmod`) is a zip with `horizon-mod.toml` at the
//!   root, the converted assets and data, the `data/<id>/horizon/`
//!   declarations, and a `bridge.json` sidecar recording where it came from.
//! * A **Java Edition pack** is what the game itself loads: `pack.mcmeta` plus
//!   `assets/` and `data/`. This is only a resource/data pack — a `pack.mcmeta`
//!   cannot carry a block registration, so a pack built here *retextures*
//!   existing blocks and adds data; it does not add a mod's blocks.
//!
//! Nothing here assumes one output works on another version: the manifest
//! records the exact target, and the pack's `pack.mcmeta` is written for one
//! pack-format pair.

use crate::convert::{Note, Outcome};
use crate::horizon::Manifest;
use crate::validate::Package;
use hb_versions::{Layout, Release};
use std::collections::BTreeMap;

/// What to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// A Horizon `.hmod`: only for 26.x, whose content format this tool writes.
    Horizon,
    /// A Java Edition resource/data pack: `pack.mcmeta` plus `assets/`/`data/`.
    JavaPack,
}

impl Product {
    pub fn label(self) -> &'static str {
        match self {
            Product::Horizon => "Horizon .hmod",
            Product::JavaPack => "Java Edition resource/data pack",
        }
    }

    /// Whether this product can carry the target at all.
    pub fn supports(self, target: &Release) -> bool {
        match self {
            Product::Horizon => target.layout == Layout::ModernHorizon,
            // 1.13+ reads a pack with `pack.mcmeta` `pack_format` 4 onwards; a
            // 1.8.9 pack needs format 1 plus `assets/` only, which the
            // converter already handles.
            Product::JavaPack => true,
        }
    }
}

/// The result of assembling a package.
#[derive(Debug, Clone)]
pub struct Assembled {
    pub package: Package,
    /// File name to write, `example-port-26.3.hmod`.
    pub file_name: String,
    /// The manifest, when one was written.
    pub manifest: Option<Manifest>,
    /// Anything the assembly had to decide, reported like every other step.
    pub notes: Vec<Note>,
}

/// Assembles the package for a target and product.
pub fn assemble(
    outcome: &Outcome,
    manifest: &Manifest,
    target: &Release,
    product: Product,
) -> Assembled {
    let mut notes = Vec::new();
    let mut package = Package::default();

    // Content, as converted. The declarations live under `data/<id>/horizon/`
    // and belong only to the Horizon product.
    for (path, bytes) in &outcome.files {
        if product == Product::JavaPack && path.starts_with(&format!("data/{}/horizon/", manifest.id))
        {
            continue;
        }
        package.files.insert(path.clone(), bytes.clone());
    }

    match product {
        Product::Horizon => {
            package.files.insert(
                "horizon-mod.toml".to_owned(),
                manifest.to_toml().into_bytes(),
            );
            let provenance = provenance_of(manifest, target, outcome);
            package.files.insert(
                "bridge.json".to_owned(),
                hb_json::to_string_pretty(&manifest.sidecar_json(&provenance)).into_bytes(),
            );
            notes.push(Note::info(
                "package.manifest.written",
                format!(
                    "`horizon-mod.toml` written for API {} with side `{}`; the game refuses \
                     unknown keys, so what the manifest cannot hold is in `bridge.json`",
                    crate::horizon::API,
                    manifest.side.as_str()
                ),
            ));
        }
        Product::JavaPack => {
            let meta = pack_mcmeta(manifest, target);
            package
                .files
                .insert("pack.mcmeta".to_owned(), hb_json::to_string_pretty(&meta).into_bytes());
            // A Java pack has no way to register a block; say so rather than
            // let the file look like a mod.
            if outcome.declares_content {
                notes.push(Note::warn(
                    "package.java_pack.content_only",
                    "this pack has `pack.mcmeta` and no code. The game loads it as a \
                     resource/data pack: it retextures and adds data, but a pack cannot \
                     register a block or an item. The Horizon package is the one that adds \
                     them",
                ));
            }
            notes.push(Note::info(
                "package.pack_mcmeta.written",
                format!(
                    "`pack.mcmeta` written for resource format {} and data format {} ({})",
                    target.resource_format.to_string_short(),
                    target.data_format.to_string_short(),
                    target.id
                ),
            ));
        }
    }

    // A README inside the package would be a file the game does not expect;
    // provenance goes in `bridge.json` instead, which the game ignores.

    let file_name = output_name(&manifest.id, &manifest.version, target, product);
    Assembled {
        package,
        file_name,
        manifest: Some(manifest.clone()),
        notes,
    }
}

fn provenance_of(
    manifest: &Manifest,
    target: &Release,
    outcome: &Outcome,
) -> BTreeMap<String, String> {
    let mut provenance = BTreeMap::new();
    provenance.insert(
        "tool".to_owned(),
        format!("zorowarrior-horizon-bridge {}", crate::VERSION),
    );
    if !outcome.source_id.is_empty() {
        provenance.insert("source_mod".to_owned(), outcome.source_id.clone());
    }
    if let Some(source) = outcome.source_version {
        provenance.insert("source_game".to_owned(), source.to_owned());
    }
    if !outcome.source_loader.is_empty() {
        provenance.insert("source_loader".to_owned(), outcome.source_loader.clone());
    }
    if !outcome.source_sha256.is_empty() {
        provenance.insert("source_jar_sha256".to_owned(), outcome.source_sha256.clone());
    }
    provenance.insert("target_game".to_owned(), target.id.to_owned());
    provenance.insert("mod_id".to_owned(), manifest.id.clone());
    provenance
}

/// `pack.mcmeta` for the target.
///
/// Since 1.21.9 the game reads `min_format`/`max_format` and ignores
/// `pack_format`; before that it is the other way round, and a pack that writes
/// the wrong one is rejected ("made for a newer version"). So both are written
/// only where the target reads them.
fn pack_mcmeta(manifest: &Manifest, target: &Release) -> hb_json::Value {
    let description = if manifest.description.is_empty() {
        manifest.name.clone()
    } else {
        manifest.description.clone()
    };
    let mut pack = hb_json::Object::new();
    if target.min_max_format {
        // A single-release pack: min and max are the same, so the game applies
        // it to exactly one version and says so in the pack list.
        pack.insert("min_format", target.resource_format.to_json(false));
        pack.insert("max_format", target.resource_format.to_json(false));
        pack.insert("description", hb_json::Value::str(description));
    } else {
        pack.insert("pack_format", target.resource_format.to_json(true));
        pack.insert("description", hb_json::Value::str(description));
    }
    hb_json::Value::obj([("pack", hb_json::Value::Obj(pack))])
}

/// A file name that says which target and product it is, because one jar can
/// produce several packages and they are not interchangeable.
pub fn output_name(id: &str, version: &str, target: &Release, product: Product) -> String {
    // `.hmod` is the extension Horizon's own tooling writes and the one the
    // Mods screen looks for; the Java pack is a plain `.zip` because the game
    // takes either but players expect a zip.
    match product {
        Product::Horizon => format!("{id}-{version}-mc{}.hmod", target.id),
        Product::JavaPack => format!("{id}-{version}-mc{}-java-pack.zip", target.id),
    }
}

/// Writes a `.hmod` (or a Java pack zip) to disk.
pub fn write(assembled: &Assembled, directory: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let entries: Vec<hb_zip::OutEntry> = assembled
        .package
        .files
        .iter()
        .map(|(name, bytes)| hb_zip::OutEntry::new(name.clone(), bytes.clone()))
        .collect();
    let bytes = hb_zip::build(&entries).map_err(|e| format!("cannot build the package: {e}"))?;
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("{}: {e}", directory.display()))?;
    let path = directory.join(&assembled.file_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// A summary in the same note vocabulary as the conversion, for a report.
pub fn summarize(assembled: &Assembled, target: &Release, product: Product) -> Vec<Note> {
    let mut notes = assembled.notes.clone();
    notes.push(Note::info(
        "package.built",
        format!(
            "{} for Minecraft {}: {} file(s), {} bytes",
            product.label(),
            target.id,
            assembled.package.len(),
            assembled.package.total_bytes()
        ),
    ));
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::Outcome;
    use crate::horizon::{Manifest, Side};

    fn outcome_with(files: &[(&str, &[u8])]) -> Outcome {
        let mut outcome = Outcome::default();
        outcome.source_id = "example".to_owned();
        outcome.source_version = Some("1.21.1");
        for (path, bytes) in files {
            outcome.files.insert(path.to_string(), bytes.to_vec());
        }
        outcome
    }

    fn manifest() -> Manifest {
        let mut manifest = Manifest::new("example_port", "Example", "1.0.0");
        manifest.description = "A converted mod".into();
        manifest.side = Side::Both;
        manifest.target_game = "26.3".into();
        manifest
    }

    #[test]
    fn a_horizon_package_has_a_manifest_sidecar_and_declarations() {
        let outcome = outcome_with(&[
            ("assets/example_port/lang/en_us.json", br#"{"a":"b"}"#),
            (
                "data/example_port/horizon/block/ruby.json",
                br#"{"template":"minecraft:stone"}"#,
            ),
        ]);
        let assembled = assemble(
            &outcome,
            &manifest(),
            hb_versions::horizon_target(),
            Product::Horizon,
        );
        assert!(assembled.package.files.contains_key("horizon-mod.toml"));
        assert!(assembled.package.files.contains_key("bridge.json"));
        assert!(assembled
            .package
            .files
            .contains_key("data/example_port/horizon/block/ruby.json"));
        let sidecar = hb_json::parse_bytes(assembled.package.files.get("bridge.json").unwrap())
            .unwrap();
        assert_eq!(sidecar.get_str("target_game"), Some("26.3"));
        assert_eq!(
            sidecar.get("provenance").unwrap().get_str("source_mod"),
            Some("example")
        );
        assert!(assembled.file_name.ends_with(".hmod"));
        assert!(assembled.file_name.contains("26.3"));
    }

    #[test]
    fn a_java_pack_has_pack_mcmeta_and_no_declarations() {
        let outcome = outcome_with(&[
            ("assets/example_port/lang/en_us.json", br#"{"a":"b"}"#),
            (
                "data/example_port/horizon/block/ruby.json",
                br#"{"template":"minecraft:stone"}"#,
            ),
        ]);
        let target = hb_versions::release("1.21.1").unwrap();
        let assembled = assemble(&outcome, &manifest(), target, Product::JavaPack);
        assert!(assembled.package.files.contains_key("pack.mcmeta"));
        assert!(!assembled.package.files.contains_key("horizon-mod.toml"));
        assert!(!assembled
            .package
            .files
            .keys()
            .any(|p| p.contains("/horizon/")));
        let meta = hb_json::parse_bytes(assembled.package.files.get("pack.mcmeta").unwrap()).unwrap();
        assert_eq!(meta.get("pack").unwrap().get("pack_format").unwrap().as_i64(), Some(34));
    }

    #[test]
    fn the_pack_format_key_follows_the_target_era() {
        // 1.21.9+ reads min_format/max_format; earlier versions read
        // pack_format. Writing both would make one of them a lie.
        let outcome = outcome_with(&[]);
        for (version, min_max) in [("1.21.1", false), ("1.21.9", true), ("26.3", true)] {
            let target = hb_versions::release(version).unwrap();
            let assembled = assemble(&outcome, &manifest(), target, Product::JavaPack);
            let meta = hb_json::parse_bytes(assembled.package.files.get("pack.mcmeta").unwrap()).unwrap();
            let pack = meta.get("pack").unwrap();
            assert_eq!(pack.has("pack_format"), !min_max, "{version}");
            assert_eq!(pack.has("min_format"), min_max, "{version}");
        }
    }

    #[test]
    fn names_say_which_target_they_are() {
        assert_eq!(
            output_name("x", "1.0", hb_versions::horizon_target(), Product::Horizon),
            "x-1.0-mc26.3.hmod"
        );
        assert_eq!(
            output_name(
                "x",
                "1.0",
                hb_versions::release("1.21.1").unwrap(),
                Product::JavaPack
            ),
            "x-1.0-mc1.21.1-java-pack.zip"
        );
    }

    #[test]
    fn horizon_packages_need_a_26_x_target() {
        assert!(Product::Horizon.supports(hb_versions::horizon_target()));
        assert!(!Product::Horizon.supports(hb_versions::release("1.21.1").unwrap()));
    }
}
