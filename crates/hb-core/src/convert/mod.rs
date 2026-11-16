//! Converting a jar's passive content into a target version's formats.
//!
//! # Shape of the conversion
//!
//! Conversion is a set of named, ordered steps, each of which either rewrites
//! a file or records why it could not. Nothing is dropped silently: every file
//! that does not reach the package appears in [`Outcome::unsupported`] or
//! [`Outcome::notes`] with a code and a reason.
//!
//! The steps mirror what each version actually reads:
//!
//! | From | To | What changes |
//! | --- | --- | --- |
//! | 1.8.9 legacy | 1.13+ | `textures/blocks` -> `textures/block`, `.lang` -> JSON, numeric model refs -> names |
//! | 1.13-1.20.6 | 1.21 | `recipes`->`recipe`, `loot_tables`->`loot_table`, `tags/items`->`tags/item`, ingredient objects -> strings |
//! | 1.21-1.21.3 | 1.21.4+ | item models gain `items/<name>.json` definitions |
//! | 1.21.1 | 26.3 | recipe `item`->`id`, typed blockstate values, loot `conditions`->`condition`, `functions`->`modifier`, named tool conditions, typed number providers, entity predicate keys |
//!
//! The 26.3 rules are the ones Horizon's own `horizon-port-pack` implements;
//! where this tool cannot reproduce a rule (worldgen rewrite, mixin
//! reimplementation) it says so instead of approximating.

use crate::jar::{Jar, JarEntry, ResourceKind};
use hb_json::{patch::Editor, Object, Value};
use hb_versions::{Layout, Release};
use std::collections::{BTreeMap, BTreeSet};

mod data;
mod lang;
mod models;
mod png;
mod templates;
mod textures;

pub use data::*;
pub use models::*;
pub use textures::*;

/// How serious a note is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The conversion changed something; worth listing in `--verbose`.
    Info,
    /// Content that will not exist in the target, with the reason.
    Warning,
    /// The port is not equivalent and the player would notice.
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "unsupported",
        }
    }
}

/// One finding: a conversion, a refusal, or a limitation.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub severity: Severity,
    /// Stable identifier, `area.thing.action`.
    pub code: &'static str,
    pub message: String,
    /// The jar entry or output file this concerns.
    pub file: Option<String>,
}

impl Note {
    pub fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            code,
            message: message.into(),
            file: None,
        }
    }

    pub fn warn(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            file: None,
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            file: None,
        }
    }

    pub fn at(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }
}

/// Options for one conversion.
#[derive(Debug, Clone)]
pub struct Options {
    /// The Horizon mod id (also the output namespace).
    pub mod_id: String,
    /// The release whose formats the package's files are in.
    pub target: &'static Release,
    /// The release the jar was made for, when it could be determined.
    pub source: Option<&'static Release>,
    /// Keep files that could not be converted under `bridge-originals/`, so a
    /// mod author can finish the port by hand. Never loaded by the game.
    pub keep_unconverted: bool,
}

impl Options {
    pub fn new(mod_id: &str, target: &'static Release, source: Option<&'static Release>) -> Self {
        Self {
            mod_id: mod_id.to_owned(),
            target,
            source,
            keep_unconverted: false,
        }
    }
}

/// The result of converting a jar.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Output path -> bytes, relative to the package root.
    pub files: BTreeMap<String, Vec<u8>>,
    pub notes: Vec<Note>,
    /// Per-input-file patch records, so `--report-json` can show exactly what
    /// changed.
    pub patches: BTreeMap<String, Vec<hb_json::patch::Patch>>,
    pub namespaces: BTreeSet<String>,
    /// True when blocks or items were declared, which forces `side = "both"`.
    pub declares_content: bool,
    /// Block templates guessed from Blockstate properties, for the report:
    /// they are a guess, not a fact from the game.
    pub guessed_templates: Vec<(String, String)>,
    /// Files copied unchanged.
    pub copied: usize,
    /// Files rewritten.
    pub converted: usize,
    /// Where the content came from: the jar's mod id, its detected game
    /// version and loader, and the jar's own hash. Carried into the package's
    /// `bridge.json` so a player can tell what a package was made from.
    pub source_id: String,
    pub source_version: Option<&'static str>,
    pub source_loader: String,
    pub source_sha256: String,
}

impl Outcome {
    fn add(&mut self, path: String, bytes: Vec<u8>) {
        self.files.insert(path, bytes);
    }

    pub fn unsupported(&self) -> impl Iterator<Item = &Note> {
        self.notes
            .iter()
            .filter(|n| n.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Note> {
        self.notes
            .iter()
            .filter(|n| n.severity == Severity::Warning)
    }

    fn note(&mut self, note: Note) {
        // Identical notes (same code, message and file) are recorded once: a
        // thousand recipes failing the same way is one finding, not a thousand.
        if !self
            .notes
            .iter()
            .any(|n| n.code == note.code && n.message == note.message && n.file == note.file)
        {
            self.notes.push(note);
        }
    }
}

/// Converts a jar's passive content. Coding errors surface as notes, never as
/// a panic; an unreadable file is skipped with its reason.
pub fn convert(jar: &Jar, options: &Options) -> Outcome {
    let mut outcome = Outcome {
        // Provenance, filled as far as it is known: a package that says what
        // it was made from is one a player can trust.
        source_id: jar.info().map(|i| i.id.clone()).unwrap_or_default(),
        source_version: options.source.map(|s| s.id),
        source_loader: jar
            .info()
            .map(|i| i.loader.id().to_owned())
            .unwrap_or_default(),
        source_sha256: jar.sha256.clone(),
        ..Outcome::default()
    };
    let Some(info) = jar.info() else {
        outcome.note(Note::error(
            "jar.metadata.missing",
            "the jar carries no mod metadata (neoforge.mods.toml, mods.toml, fabric.mod.json \
             or mcmod.info), so it is not a mod jar this tool can port",
        ));
        return outcome;
    };
    outcome.note(Note::info(
        "jar.metadata.loader",
        format!(
            "read {} metadata: id `{}`, version `{}`",
            info.loader.id(),
            info.id,
            info.version
        ),
    ));

    let target = options.target;
    let source_layout = options.source.map(|s| s.layout).unwrap_or(Layout::SingularFolders);

    // Java-only functionality: reported before the content, because it decides
    // whether the port is "works", "partly" or "doesn't run".
    report_java(jar, &mut outcome);

    // Namespaces: the mod's own plus any it writes files into. The package id
    // is the mod id, so a foreign namespace has to be declared in the
    // manifest or its files cannot be carried.
    // Every namespace the mod writes into becomes one this package owns, so a
    // file in `assets/<ns>/` is carried under the package id rather than left
    // behind or colliding with another mod's. Nothing foreign is carried: a
    // mod's files are its own.
    let owned: std::collections::BTreeSet<String> = jar
        .namespaces()
        .into_iter()
        .filter(|ns| !crate::horizon::RESERVED_NAMESPACES.contains(&ns.as_str()))
        .collect();
    let namespaces = owned.clone();
    let mut declared = owned.clone();
    declared.remove(&info.id);
    declared.remove(&options.mod_id);
    outcome.namespaces = declared.clone();
    let foreign: Vec<String> = declared.iter().cloned().collect();
    if !foreign.is_empty() {
        outcome.note(Note::info(
            "asset.namespace.merged",
            format!(
                "the jar also writes into {}; those files move under the package id so the \
                 package is self-contained and cannot collide with another mod",
                foreign.join(", ")
            ),
        ));
    }

    let mut entries: Vec<&JarEntry> = jar.entries.iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    for entry in entries {
        let kind = match entry.kind {
            Some(kind) => kind,
            None => continue,
        };
        let translated = match rewrite_path(&entry.name, kind, source_layout, target.layout) {
            PathOutcome::Keep(path) => path,
            PathOutcome::Rename(path, code) => {
                outcome.note(
                    Note::info(code, format!("{} -> {}", entry.name, path)).at(entry.name.clone()),
                );
                path
            }
            PathOutcome::Drop(code, why) => {
                outcome.note(Note::warn(code, why).at(entry.name.clone()));
                continue;
            }
        };
        // Rename the namespace segment to the package's id when the files are
        // the mod's own; extra namespaces keep their names and are declared.
        let output = retarget_namespace(&translated, &entry.name, &info.id, &options.mod_id);

        match kind {
            ResourceKind::Texture => {
                match png::check(&entry.bytes) {
                    Ok(info_png) => {
                        if let Some(warning) = info_png.warning() {
                            outcome.note(
                                Note::warn("asset.texture.odd_size", warning)
                                    .at(entry.name.clone()),
                            );
                        }
                        // Not square, or a multiple of 16 that no vanilla
                        // texture uses: legal, but the usual cause of a
                        // texture that renders at the wrong scale.
                        if !info_png.is_square() {
                            outcome.note(
                                Note::info(
                                    "asset.texture.not_square",
                                    format!(
                                        "{}x{}: vanilla textures are square, and a sprite whose \
                                         proportions differ is drawn stretched",
                                        info_png.width, info_png.height
                                    ),
                                )
                                .at(entry.name.clone()),
                            );
                        }
                        if info_png.resolution() > 16 {
                            outcome.note(
                                Note::info(
                                    "asset.texture.high_resolution",
                                    format!(
                                        "{}x{} is {}x resolution, which needs the pack to declare \
                                         it or the game draws it at 1x",
                                        info_png.width,
                                        info_png.height,
                                        info_png.resolution()
                                    ),
                                )
                                .at(entry.name.clone()),
                            );
                        }
                    }
                    Err(reason) => {
                        outcome.note(
                            Note::error(
                                "asset.texture.invalid_png",
                                format!("not a readable PNG, so the game would refuse it: {reason}"),
                            )
                            .at(entry.name.clone()),
                        );
                        continue;
                    }
                }
                outcome.add(output, entry.bytes.clone());
                outcome.copied += 1;
            }
            ResourceKind::TextureMeta | ResourceKind::SoundFile | ResourceKind::StructureNbt => {
                outcome.add(output, entry.bytes.clone());
                outcome.copied += 1;
            }
            ResourceKind::LangLegacy => {
                match lang::convert_legacy(&entry.bytes) {
                    Ok(json) => {
                        // 1.13+ reads `lang/en_us.json`: the locale is
                        // lowercase, and `en_US.json` is not loaded at all.
                        let json_path = output
                            .replace(".lang", ".json")
                            .rsplit_once('/')
                            .map(|(dir, file)| format!("{dir}/{}", file.to_lowercase()))
                            .unwrap_or_else(|| output.replace(".lang", ".json"));
                        outcome.note(
                            Note::info(
                                "asset.lang.legacy_to_json",
                                "1.8.9 `.lang` converted to the JSON form 1.13+ reads",
                            )
                            .at(entry.name.clone()),
                        );
                        outcome.add(json_path, json);
                        outcome.converted += 1;
                    }
                    Err(reason) => {
                        outcome.note(
                            Note::warn(
                                "asset.lang.legacy_unreadable",
                                format!("cannot convert the .lang file: {reason}"),
                            )
                            .at(entry.name.clone()),
                        );
                    }
                }
            }
            kind if kind.is_text() => {
                let (bytes, changed, patches) = match convert_json(
                    &entry.name,
                    &entry.bytes,
                    kind,
                    source_layout,
                    target,
                    &options.mod_id,
                    &info.id,
                    &namespaces,
                    &mut outcome,
                ) {
                    Some(result) => result,
                    None => continue,
                };
                if changed {
                    outcome.converted += 1;
                } else {
                    outcome.copied += 1;
                }
                if !patches.is_empty() {
                    outcome.patches.insert(entry.name.clone(), patches);
                }
                outcome.add(output, bytes);
            }
            _ => {
                outcome.add(output, entry.bytes.clone());
                outcome.copied += 1;
            }
        }
    }

    // Content declarations. A Horizon content mod declares each block and item
    // in `data/<id>/horizon/...`; the templates are guessed from what the
    // blockstates and item models say, and the report says they are guesses.
    let declarations = templates::declare(&mut outcome, &options.mod_id, jar, target);
    if declarations > 0 {
        outcome.declares_content = true;
        outcome.note(Note::info(
            "content.declarations",
            format!(
                "{declarations} block/item declaration(s) written; each copies a vanilla template \
                 guessed from the blockstate's properties, so check them against the game"
            ),
        ));
    }

    if options.keep_unconverted {
        for (name, reason) in &jar.skipped {
            outcome.add(
                format!("bridge-originals/{}", sanitize(name)),
                jar.find(name).map(|e| e.bytes.clone()).unwrap_or_default(),
            );
            outcome.note(
                Note::warn(
                    "jar.entry.kept",
                    format!("kept under bridge-originals/ because it did not convert: {reason}"),
                )
                .at(name.clone()),
            );
        }
    }

    if !outcome
        .files
        .iter()
        .any(|(p, _)| p.starts_with("assets/") || p.starts_with("data/"))
    {
        outcome.note(Note::error(
            "content.empty",
            "nothing in this jar could be carried: it has no assets or data a content mod may \
             hold. Its behaviour is all Java code, which Horizon does not run",
        ));
    }
    outcome
}

/// What to do with a jar path for a target layout.
enum PathOutcome {
    Keep(String),
    Rename(String, &'static str),
    Drop(&'static str, String),
}

/// The 1.21 folder renames, the 1.13 texture folder renames and 26.x's own
/// moves, as `(kind, from-suffix, to-suffix)` on the part after the namespace.
const RENAMES: &[(&str, &str, &str)] = &[
    // 1.13 flattening: plural texture folders became singular.
    ("textures/blocks/", "textures/block/", "asset.path.flattened"),
    ("textures/items/", "textures/item/", "asset.path.flattened"),
    // 1.21 renamed the data folders to the singular (pack format 45).
    ("recipes/", "recipe/", "data.path.singular_folder"),
    ("loot_tables/", "loot_table/", "data.path.singular_folder"),
    ("advancements/", "advancement/", "data.path.singular_folder"),
    ("functions/", "function/", "data.path.singular_folder"),
    ("structures/", "structure/", "data.path.singular_folder"),
    ("predicates/", "predicate/", "data.path.singular_folder"),
    ("item_modifiers/", "item_modifier/", "data.path.singular_folder"),
    ("tags/blocks/", "tags/block/", "data.path.singular_tag_folder"),
    ("tags/items/", "tags/item/", "data.path.singular_tag_folder"),
    (
        "tags/entity_types/",
        "tags/entity_type/",
        "data.path.singular_tag_folder",
    ),
    ("tags/fluids/", "tags/fluid/", "data.path.singular_tag_folder"),
    (
        "tags/game_events/",
        "tags/game_event/",
        "data.path.singular_tag_folder",
    ),
    (
        "tags/functions/",
        "tags/function/",
        "data.path.singular_tag_folder",
    ),
];

fn rewrite_path(
    name: &str,
    kind: ResourceKind,
    source: Layout,
    target: Layout,
) -> PathOutcome {
    let (root, namespace, tail) = match split_namespace(name) {
        Some(parts) => parts,
        None => return PathOutcome::Keep(name.to_owned()),
    };
    let mut tail = tail.to_owned();
    if root == "assets" && target == Layout::Legacy {
        // Going *to* 1.8.9: models and blockstates do not exist there; only
        // textures and lang survive, under their legacy folders.
        if kind == ResourceKind::BlockState
            || kind == ResourceKind::BlockModel
            || kind == ResourceKind::ItemModel
            || kind == ResourceKind::ItemDefinition
        {
            return PathOutcome::Drop(
                "asset.path.legacy_needs_models",
                "1.8.9 has no blockstate or model JSON for a mod's own block: blocks are code \
                 (a `Block` subclass registered in `GameRegistry`), so the model cannot be carried"
                    .into(),
            );
        }
        tail = match kind {
            ResourceKind::Texture => {
                if let Some(stripped) = tail.strip_prefix("textures/block/") {
                    format!("textures/blocks/{stripped}")
                } else if let Some(stripped) = tail.strip_prefix("textures/item/") {
                    format!("textures/items/{stripped}")
                } else {
                    tail
                }
            }
            _ => tail,
        };
        if kind == ResourceKind::Recipe || kind == ResourceKind::LootTable {
            return PathOutcome::Drop(
                "data.path.legacy_json",
                "1.8.9 reads no recipe or loot JSON: both are Java code and NBT tables in the \
                 world save"
                    .into(),
            );
        }
        return PathOutcome::Keep(format!("{root}/{namespace}/{tail}"));
    }
    // 1.13+ is the common case; apply only the renames the target needs.
    if target == Layout::Flattened {
        for (from, to, code) in RENAMES {
            if let Some(rest) = tail.strip_prefix(from) {
                let renamed = format!("{to}{rest}");
                if renamed != tail {
                    return PathOutcome::Rename(
                        format!("{root}/{namespace}/{renamed}"),
                        code,
                    );
                }
            }
        }
        return PathOutcome::Keep(format!("{root}/{namespace}/{tail}"));
    }
    let mut renames = Vec::new();
    for (from, to, code) in RENAMES {
        // Only the floored fixes apply: from a legacy layout both sets do, from
        // 1.13-1.20.6 only the 1.21 folder names, from 1.21 on nothing.
        let applies = match (source, *from) {
            // A legacy jar's files are 1.8 names, so both the 1.13 texture move
            // and the 1.21 folder names change them.
            (Layout::Legacy, _) => true,
            // A 1.13-1.20.6 jar already uses the 1.13 texture folders; only the
            // 1.21 renames apply.
            (Layout::Flattened, "textures/blocks/") | (Layout::Flattened, "textures/items/") => {
                false
            }
            (Layout::Flattened, _) => true,
            // 1.21.x names its folders the way 1.21+ does; nothing to rename.
            _ => false,
        };
        if applies {
            if let Some(rest) = tail.strip_prefix(from) {
                let renamed = format!("{to}{rest}");
                if renamed != tail {
                    renames.push((renamed, *code));
                }
            }
        }
    }
    if let Some((renamed, code)) = renames.pop() {
        return PathOutcome::Rename(format!("{root}/{namespace}/{renamed}"), code);
    }
    PathOutcome::Keep(format!("{root}/{namespace}/{tail}"))
}

/// Rewrites the namespace segment of a path from the jar's own id to the
/// package id; other namespaces are left alone and declared in the manifest.
fn retarget_namespace(path: &str, original: &str, source_id: &str, package_id: &str) -> String {
    let Some((root, namespace, tail)) = split_namespace(path) else {
        return path.to_owned();
    };
    // Game-owned namespaces stay as they are; anything else is the mod's own
    // (or a helper namespace it writes into) and moves under the package id.
    if namespace == "minecraft" || namespace == "c" || source_id == package_id {
        return path.to_owned();
    }
    let _ = original;
    let _ = root;
    format!("{root}/{package_id}/{tail}")
}

/// Splits `assets/ns/rest` or `data/ns/rest`.
pub fn split_namespace(name: &str) -> Option<(&str, &str, &str)> {
    let (root, rest) = name.split_once('/')?;
    if root != "assets" && root != "data" {
        return None;
    }
    let (namespace, tail) = rest.split_once('/')?;
    if namespace.is_empty() || tail.is_empty() {
        return None;
    }
    Some((root, namespace, tail))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' => '/',
            c if c.is_ascii_alphanumeric() || "._-".contains(c) => c,
            _ => '_',
        })
        .collect()
}

/// Reports Java-only functionality that a content mod cannot reproduce.
fn report_java(jar: &Jar, outcome: &mut Outcome) {
    let classes: Vec<&hb_classfile::ClassFile> = jar.classes.iter().collect();
    let census = hb_classfile::census(classes.into_iter());

    for entry in &census.entry_points {
        outcome.note(Note::warn(
            "java.entry_point",
            format!(
                "{entry}: entry points are Java executed by the loader, which Horizon does not run"
            ),
        ));
    }
    for (config, classes) in hb_classfile::summarize_mixins(&jar.mixin_configs) {
        outcome.note(Note::warn(
            "java.mixin",
            format!(
                "{} names {} mixin class(es) ({}); Mixins rewrite game classes at load time and \
                 cannot be carried. Their effect needs a reimplementation through Horizon's mod API",
                config,
                classes.len(),
                classes.join(", ")
            ),
        ));
    }
    for behaviour in &census.behaviour {
        outcome.note(Note::warn(
            "java.behaviour",
            format!("{behaviour}: the Java behaviour does not port, its content may"),
        ));
    }
    for library in &census.libraries {
        let label = hb_classfile::describe_library(library)
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        outcome.note(Note::warn(
            "java.library",
            format!(
                "depends on `{library}`{label}: Java library code that must be reimplemented or \
                 dropped, and the mod will not behave as it does on Java"
            ),
        ));
    }
    if !jar.services.is_empty() {
        for (service, providers) in &jar.services {
            outcome.note(Note::warn(
                "java.service_loader",
                format!(
                    "services/{service} declares {} implementation(s) loaded by the JVM",
                    providers.len()
                ),
            ));
        }
    }
    if let Some(main) = &jar.main_class {
        outcome.note(Note::warn(
            "java.main_class",
            format!("manifest Main-Class `{main}`: a Java entry point"),
        ));
    }
    if census.registries.is_empty() {
        outcome.note(Note::info(
            "java.registry.none",
            "no registry classes referenced from bytecode: the mod's content is declared only by \
             its data files",
        ));
    } else {
        outcome.note(Note::info(
            "java.registry.found",
            format!(
                "registers through {} registry class(es), which a content mod replaces with \
                 `data/<id>/horizon/` declarations",
                census.registries.len()
            ),
        ));
    }
}

/// Rewrites one JSON file for the target version. Returns `(bytes, changed,
/// patches)`, or `None` when the file was refused and reported.
#[allow(clippy::too_many_arguments)]
fn convert_json(
    name: &str,
    bytes: &[u8],
    kind: ResourceKind,
    source: Layout,
    target: &'static Release,
    package_id: &str,
    source_id: &str,
    namespaces: &std::collections::BTreeSet<String>,
    outcome: &mut Outcome,
) -> Option<(Vec<u8>, bool, Vec<hb_json::patch::Patch>)> {
    let mut value = match hb_json::parse_bytes(bytes) {
        Ok(value) => value,
        Err(e) => {
            outcome.note(
                Note::error(
                    "json.unreadable",
                    format!(
                        "the game skips a file it cannot read, so this one is not carried: {e}"
                    ),
                )
                .at(name.to_owned()),
            );
            return None;
        }
    };
    let mut editor = Editor::new();
    // 26.x dropped `minecraft:chain` for `minecraft:iron_chain`; a data file
    // naming the old id fails to load. This is data-independent of the layout.
    if target.layout == Layout::ModernHorizon {
        let mut renamed = 0;
        rename_ids(&mut value, &mut renamed);
        if renamed > 0 {
            outcome.note(
                Note::info(
                    "data.id.renamed",
                    format!("{renamed} reference(s) to `minecraft:chain` became `minecraft:iron_chain` (26.x)"),
                )
                .at(name.to_owned()),
            );
        }
    }
    match kind {
        ResourceKind::Recipe => data::recipe(&mut value, &mut editor, source, target, outcome, name),
        ResourceKind::LootTable => data::loot(&mut value, &mut editor, source, target, outcome, name),
        ResourceKind::Advancement => {
            data::advancement(&mut value, &mut editor, source, target, outcome, name)
        }
        ResourceKind::Tag => data::tag(&mut value, &mut editor, target, outcome, name),
        ResourceKind::BlockState => {
            models::blockstate(&mut value, &mut editor, source, target, outcome, name)
        }
        ResourceKind::BlockModel | ResourceKind::ItemModel => {
            models::model(
                &mut value,
                &mut editor,
                source,
                target,
                outcome,
                name,
                kind,
                package_id,
            )
        }
        ResourceKind::ItemDefinition => {
            models::item_definition(&mut value, &mut editor, target, outcome, name)
        }
        ResourceKind::Lang => {
            lang::modern(&mut value, &mut editor, outcome, name);
        }
        _ => {}
    }
    // Rename any namespaced id in an owned namespace to the package id, so a
    // mod depends on nothing but itself. Vanilla ids are untouched.
    if source_id != package_id {
        retarget_ids(&mut value, namespaces, package_id, &mut editor, name);
    }
    let patches = editor.into_patches();
    let changed = !patches.is_empty();
    let text = if changed {
        hb_json::to_string_pretty(&value)
    } else {
        // Unchanged files keep their exact bytes: a diff should show only real
        // conversions, and a byte-identical copy is provable.
        String::from_utf8_lossy(bytes).into_owned()
    };
    Some((text.into_bytes(), changed, patches))
}

/// Rewrites `minecraft:chain` to `minecraft:iron_chain` throughout a value.
fn rename_ids(value: &mut Value, count: &mut usize) {
    match value {
        Value::Str(s) => {
            if s == "minecraft:chain" {
                *s = "minecraft:iron_chain".to_owned();
                *count += 1;
            }
        }
        Value::Arr(items) => items.iter_mut().for_each(|v| rename_ids(v, count)),
        Value::Obj(object) => {
            // Object keys are ids in some places (a component map, a tag's
            // values); values are ids far more often.
            object.iter_mut().for_each(|(_, v)| rename_ids(v, count));
        }
        _ => {}
    }
}

/// Rewrites namespaced ids from the jar's own namespaces to the package id
/// inside JSON values, so a package depends on nothing but itself and nothing
/// it names points at a namespace it does not ship.
///
/// Game-owned namespaces (`minecraft:`, `c:`, `horizon:`) are left alone: they
/// belong to the game, not to the mod.
fn retarget_ids(
    value: &mut Value,
    namespaces: &std::collections::BTreeSet<String>,
    to: &str,
    editor: &mut Editor,
    file: &str,
) {
    let mut prefixes: Vec<String> = namespaces
        .iter()
        .filter(|ns| !crate::horizon::RESERVED_NAMESPACES.contains(&ns.as_str()))
        .map(|ns| format!("{ns}:"))
        .collect();
    prefixes.sort();
    if prefixes.is_empty() {
        return;
    }
    fn walk(
        value: &mut Value,
        prefixes: &[String],
        to: &str,
        editor: &mut Editor,
        file: &str,
        path: &str,
    ) {
        match value {
            Value::Str(s) => {
                let Some((prefix, rest)) = prefixes
                    .iter()
                    .find_map(|prefix| s.strip_prefix(prefix.as_str()).map(|r| (prefix, r)))
                else {
                    return;
                };
                let rest = rest.to_owned();
                let prefix = prefix.clone();
                let new = format!("{to}:{rest}");
                editor.record(
                    "data.id.retarget_namespace",
                    path,
                    hb_json::patch::Node::Present(Value::str(format!("{prefix}{rest}"))),
                    hb_json::patch::Node::Present(Value::str(new.clone())),
                    "an id in the mod's own namespace became the package id",
                );
                *s = new;
                let _ = file;
            }
            Value::Arr(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    let child = format!("{path}/{i}");
                    walk(item, prefixes, to, editor, file, &child);
                }
            }
            Value::Obj(object) => {
                // A key can be an id too: a tag's `values` is a list, but a
                // component map and some patch files key by id.
                let keys: Vec<String> = object.keys().map(|k| k.to_owned()).collect();
                for key in keys {
                    if let Some((prefix, rest)) = prefixes
                        .iter()
                        .find_map(|prefix| key.strip_prefix(prefix.as_str()).map(|r| (prefix, r)))
                    {
                        let rest = rest.to_owned();
                        let prefix = prefix.clone();
                        let renamed = format!("{to}:{rest}");
                        if let Some(value) = object.remove(&key) {
                            object.insert_last(renamed.clone(), value);
                            editor.record(
                                "data.id.retarget_namespace",
                                &format!("{path}/{}", hb_json::patch::join(&[&key])),
                                hb_json::patch::Node::Present(Value::str(key.clone())),
                                hb_json::patch::Node::Present(Value::str(renamed)),
                                "an id keyed in the mod's own namespace became the package id",
                            );
                        }
                        let _ = prefix;
                    }
                    if let Some(item) = object.get_mut(&key) {
                        let child = format!("{path}/{}", hb_json::patch::join(&[&key]));
                        walk(item, prefixes, to, editor, file, &child);
                    }
                }
            }
            _ => {}
        }
    }
    walk(value, &prefixes, to, editor, file, "");
}

/// Builds an `Object` from pairs, for terse construction in converters.
pub(crate) fn object(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Obj(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Object>(),
    )
}

#[cfg(test)]
mod tests;
