//! Working out what a jar is: which loader made it, which Minecraft version it
//! targets, and what that means for a port.
//!
//! Everything here is evidence, not inference. A version is only *stated* when
//! the jar names it (`fabric.mod.json`'s `depends.minecraft`, `mods.toml`'s
//! `versionRange`, `mcmod.info`'s `mcversion`, a pack format in
//! `pack.mcmeta`); a class file's major version gives the Java release, which
//! narrows the Minecraft range but never decides it. When the jar is silent the
//! answer is [`Confidence::Unknown`] and the caller has to ask.

use crate::jar::{Jar, ResourceKind};
use hb_mcmeta::{Loader, ModInfo, PackMeta};
use hb_versions::{release, Layout, Release, Requirement};
use std::collections::{BTreeMap, BTreeSet};

/// How sure the detection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// The jar names exactly one release.
    Stated,
    /// The jar states a range that covers several releases.
    Narrowed,
    /// Only the Java release or the file layout narrows it.
    Guessed,
    /// No evidence at all.
    Unknown,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Stated => "stated by the jar",
            Confidence::Narrowed => "from a version range",
            Confidence::Guessed => "guessed from the Java release or file layout",
            Confidence::Unknown => "unknown",
        }
    }
}

/// What the inspection found.
#[derive(Debug, Clone)]
pub struct Detection {
    /// The loader metadata, when there was any.
    pub loader: Option<Loader>,
    pub metadata: Option<ModInfo>,
    /// The one release the jar names, when it names one.
    pub version: Option<&'static Release>,
    /// Every release its requirement covers, oldest first.
    pub versions: Vec<&'static Release>,
    pub confidence: Confidence,
    /// The highest class-file major version, e.g. 61 for Java 17.
    pub class_major: Option<u16>,
    /// `(resource, data)` pack formats from `pack.mcmeta`.
    pub pack_formats: Option<(u32, u32)>,
    /// One sentence per piece of evidence, for the report.
    pub evidence: Vec<String>,
    /// `(code, message)`: things to settle before converting.
    pub concerns: Vec<(String, String)>,
}

impl Detection {
    /// The source release to convert *from*. `None` when unknown.
    pub fn source(&self) -> Option<&'static Release> {
        self.version
    }

    pub fn loader_label(&self) -> &'static str {
        self.loader.as_ref().map(|l| l.id()).unwrap_or("none")
    }

    pub fn has_java(&self) -> bool {
        self.class_major.is_some()
    }

    /// One line for the report.
    pub fn headline(&self) -> String {
        match self.version {
            Some(v) => format!("Minecraft {} ({})", v.id, self.confidence.label()),
            None => format!("Minecraft version not stated ({})", self.confidence.label()),
        }
    }
}

/// Inspects a jar. Never fails: an unrecognisable jar is reported as such.
pub fn detect(jar: &Jar) -> Detection {
    let mut evidence = Vec::new();
    let mut concerns = Vec::new();

    let metadata = jar.info().cloned();
    match &metadata {
        Some(info) => evidence.push(format!(
            "{} metadata for mod `{}` {}",
            info.loader.id(),
            info.id,
            info.version
        )),
        None => evidence.push("no loader metadata found".to_owned()),
    }

    // Class files: presence and the highest major version.
    let mut class_major: Option<u16> = None;
    for class in &jar.classes {
        class_major = Some(class_major.map_or(class.major_version, |m| m.max(class.major_version)));
    }
    if !jar.classes.is_empty() {
        evidence.push(format!("{} Java class file(s)", jar.classes.len()));
        if let Some(major) = class_major {
            evidence.push(format!(
                "class version {major}, i.e. Java {}",
                major.saturating_sub(44)
            ));
        }
    }
    if !jar.mixin_configs.is_empty() || !jar.mixin_documents.is_empty() {
        evidence.push(format!(
            "{} mixin configuration(s) naming {} mixin class(es)",
            jar.mixin_documents.len().max(jar.mixin_configs.len()),
            jar.mixin_configs.len()
        ));
    }

    let pack_formats = jar
        .pack_meta
        .as_ref()
        .and_then(|meta: &PackMeta| pack_formats_of(meta));
    if let Some((resource, data)) = pack_formats {
        evidence.push(format!(
            "pack.mcmeta declares resource format {resource} and data format {data}"
        ));
    }

    // The version the loader metadata names.
    let mut version = None;
    let mut versions: Vec<&'static Release> = Vec::new();
    let mut confidence = Confidence::Unknown;

    if let Some(text) = metadata.as_ref().and_then(stated_version) {
        let requirement = Requirement::parse(&text);
        // Which family a requirement is about is decided by the lowest version
        // it names, which is always inside the range it describes.
        let family = leading_version(&text)
            .as_deref()
            .and_then(hb_versions::parse_version)
            .map(|r| r.family);
        let candidates: Vec<&'static Release> = hb_versions::targets()
            .filter(|r| family.is_none_or(|f| Some(f) == Some(r.family)))
            .filter(|r| requirement.accepts(r.id))
            .collect();
        versions = candidates.clone();
        match candidates.len() {
            0 => {
                evidence.push(format!(
                    "`{text}` matches no release in this build's database (the jar may target \
                     something newer)"
                ));
                concerns.push((
                    "detect.version.unknown".into(),
                    format!(
                        "the jar targets `{text}`, which this build does not know, so its formats \
                         cannot be checked against a known profile; supply --source-version"
                    ),
                ));
            }
            1 => {
                version = candidates.first().copied();
                confidence = Confidence::Stated;
                evidence.push(format!("`{text}` names exactly one release"));
            }
            _ => {
                confidence = Confidence::Narrowed;
                evidence.push(format!(
                    "`{text}` covers {} releases ({} to {})",
                    candidates.len(),
                    candidates.first().map_or("?", |r| r.id),
                    candidates.last().map_or("?", |r| r.id)
                ));
                concerns.push((
                    "detect.version.range".into(),
                    format!(
                        "the jar accepts {} releases, whose formats differ. Supply \
                         --source-version with the exact one it was built against, or accept the \
                         oldest ({}) as the source",
                        candidates.len(),
                        candidates.first().map_or("?", |r| r.id)
                    ),
                ));
            }
        }
    }

    // A pack format can name a family when it belongs to one release only.
    if version.is_none() && confidence == Confidence::Unknown {
        if let Some((resource, data)) = pack_formats {
            let candidates: Vec<&'static Release> = hb_versions::targets()
                .filter(|r| {
                    r.resource_format.major == resource && r.data_format.major == data
                })
                .collect();
            match candidates.len() {
                1 => {
                    version = candidates.first().copied();
                    versions = candidates.clone();
                    confidence = Confidence::Narrowed;
                    evidence.push("those pack formats belong to one release".to_owned());
                }
                0 => {}
                _ => {
                    versions = candidates.clone();
                    confidence = Confidence::Narrowed;
                    evidence.push(format!(
                        "those pack formats are shared by {} releases ({} to {})",
                        candidates.len(),
                        candidates.first().map_or("?", |r| r.id),
                        candidates.last().map_or("?", |r| r.id)
                    ));
                }
            }
        }
    }

    // Java release: a bound on the Minecraft range, never a decision.
    if version.is_none() && confidence == Confidence::Unknown {
        if let Some(major) = class_major {
            let (low, high, java) = java_band(major);
            let band: Vec<&'static Release> = hb_versions::targets()
                .filter(|r| within(r.id, low, high))
                .collect();
            if !band.is_empty() {
                versions = band.clone();
                confidence = Confidence::Guessed;
                evidence.push(format!(
                    "Java {java} narrows the target to Minecraft {low} to {high} ({} releases)",
                    band.len()
                ));
                concerns.push((
                    "detect.version.guessed".into(),
                    format!(
                        "the jar does not name its Minecraft version; Java {java} narrows it to \
                         {low} to {high}. Pass --source-version, because formats differ across \
                         that range"
                    ),
                ));
            }
        }
    }

    // Last resort: what the file names look like.
    if version.is_none() && confidence == Confidence::Unknown {
        if let Some((why, release)) = infer_from_layout(jar) {
            evidence.push(format!("file layout ({why}) matches {}", release.id));
            version = Some(release);
            versions = vec![release];
            confidence = Confidence::Guessed;
            concerns.push((
                "detect.version.guessed".into(),
                format!(
                    "the jar's file layout ({why}) suggests {}, which may be the oldest release \
                     with that layout rather than the one the mod was built for",
                    release.id
                ),
            ));
        }
    }

    if metadata.is_none() && jar.classes.is_empty() && jar.pack_meta.is_none() {
        concerns.push((
            "detect.unrecognised".into(),
            "the jar has no loader metadata, no class files and no pack.mcmeta; it may not be a \
             mod at all"
                .into(),
        ));
    }

    Detection {
        loader: metadata.as_ref().map(|i| i.loader.clone()),
        metadata,
        version,
        versions,
        confidence,
        class_major,
        pack_formats,
        evidence,
        concerns,
    }
}

/// The Minecraft releases a Java class version is compatible with.
fn java_band(major: u16) -> (&'static str, &'static str, u16) {
    match major {
        ..=51 => ("1.8.9", "1.12.2", major.saturating_sub(44)),
        52..=55 => ("1.8.9", "1.16.5", 8),
        56..=60 => ("1.12.2", "1.16.5", major.saturating_sub(44)),
        61..=62 => ("1.18", "1.20.4", 17),
        63..=64 => ("1.20.5", "1.21.11", 21),
        _ => ("1.20.5", "26.3", 21),
    }
}

fn within(id: &str, low: &str, high: &str) -> bool {
    let key = hb_versions::version_key(id);
    key >= hb_versions::version_key(low) && key <= hb_versions::version_key(high)
}

/// `pack.mcmeta`'s formats, tolerating 1.21.9's `min_format`/`max_format`.
fn pack_formats_of(meta: &PackMeta) -> Option<(u32, u32)> {
    let resource = meta
        .pack_format
        .or(meta.min_format)
        .or(meta.max_format)?
        .major;
    let data = meta
        .min_format
        .map(|f| f.major)
        .unwrap_or(resource);
    Some((resource, data))
}

/// The first version a requirement mentions, which is always inside the range
/// it describes: `1.21.1` from `>=1.21 <1.22`, `1.21` from `[1.21,1.22)`.
fn leading_version(text: &str) -> Option<String> {
    let mut out = String::new();
    for c in text.trim().trim_start_matches(['v', 'V']).chars() {
        if c.is_ascii_digit() || c == '.' {
            out.push(c);
        } else if !out.is_empty() {
            break;
        }
    }
    let out = out.trim_end_matches('.').to_owned();
    (!out.is_empty()).then_some(out)
}

/// The requirement text a loader's metadata states for Minecraft.
fn stated_version(info: &ModInfo) -> Option<String> {
    // `ModInfo::minecraft.games` holds the `depends.minecraft` / `versionRange`
    // entries in the order the file wrote them.
    info.minecraft
        .iter()
        .map(|dependency| dependency.requirement.trim())
        .find(|requirement| !requirement.is_empty() && *requirement != "*")
        .map(|requirement| requirement.to_owned())
}

/// What the file names alone suggest, with the evidence named.
fn infer_from_layout(jar: &Jar) -> Option<(&'static str, &'static Release)> {
    let has = |prefix: &str| jar.entries.iter().any(|e| e.name.starts_with(prefix));
    let any = |needle: &str| jar.entries.iter().any(|e| e.name.contains(needle));

    if has("assets/") && any("/textures/blocks/") {
        return release("1.8.9").map(|r| ("`textures/blocks/`", r));
    }
    if has("assets/") && any("/models/item/") && any(".lang") {
        return release("1.8.9").map(|r| ("`.lang` files beside item models", r));
    }
    if has("data/") && any("/recipes/") {
        return release("1.20.1").map(|r| ("plural `recipes/` folders", r));
    }
    if has("data/") && any("/recipe/") {
        return release("1.21.1").map(|r| ("singular `recipe/` folders", r));
    }
    if has("data/") && any("/enchantment/") {
        return hb_versions::targets()
            .find(|r| r.id == "1.21.5")
            .map(|r| ("`data/<ns>/enchantment/`", r));
    }
    None
}

/// How a Java mod can fare on a target, in the words the report uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Resource-and-data only, formats handled: expected to work.
    Works,
    /// Content ports, behaviour does not.
    Partly,
    /// Only code: nothing usable.
    WontRun,
    /// No portable content at all.
    Nothing,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Works => "WORKS",
            Verdict::Partly => "PARTLY",
            Verdict::WontRun => "DOESN'T RUN",
            Verdict::Nothing => "NOTHING TO PORT",
        }
    }

    pub fn sentence(self) -> &'static str {
        match self {
            Verdict::Works => "the content converts and nothing in the mod needs code",
            Verdict::Partly => "the content converts, the mod's behaviour does not",
            Verdict::WontRun => "nothing in the mod survives without Java",
            Verdict::Nothing => "the jar contains no portable content",
        }
    }
}

/// Counts what the jar holds, by kind, for the report.
pub fn content_census(jar: &Jar) -> BTreeMap<&'static str, usize> {
    let mut counts = jar.census();
    counts.retain(|_, n| *n > 0);
    counts
}

/// The verdict and a sentence explaining it.
pub fn content_verdict(jar: &Jar, _detection: &Detection) -> (Verdict, String) {
    let mut parts = Vec::new();
    for kind in [
        ResourceKind::BlockState,
        ResourceKind::ItemModel,
        ResourceKind::ItemDefinition,
        ResourceKind::BlockModel,
        ResourceKind::Texture,
        ResourceKind::Lang,
        ResourceKind::LangLegacy,
        ResourceKind::Recipe,
        ResourceKind::LootTable,
        ResourceKind::Tag,
        ResourceKind::SoundFile,
        ResourceKind::StructureNbt,
    ] {
        let count = jar.entries_of(kind).count();
        if count > 0 {
            parts.push(format!("{count} {}", kind.label()));
        }
    }
    let content = if parts.is_empty() {
        "no assets or data".to_owned()
    } else {
        parts.join(", ")
    };
    let classes = jar.classes.len();

    if parts.is_empty() && classes == 0 {
        return (
            Verdict::Nothing,
            format!("nothing portable was found: {}", jar.entries.len()),
        );
    }
    if parts.is_empty() {
        return (
            Verdict::WontRun,
            format!(
                "all {classes} class file(s): every effect is Java, and Horizon does not run Java"
            ),
        );
    }
    if classes > 0 {
        return (
            Verdict::Partly,
            format!(
                "passive content converts ({content}); {classes} class file(s) do not, so blocks \
                 will be inert and behaviour is lost"
            ),
        );
    }
    (Verdict::Works, format!("resource-and-data only: {content}"))
}

/// Namespaces the jar writes into that are not the game's own.
pub fn own_namespaces(jar: &Jar) -> BTreeSet<String> {
    jar.namespaces()
        .into_iter()
        .filter(|ns| !crate::horizon::RESERVED_NAMESPACES.contains(&ns.as_str()))
        .collect()
}

/// The family a release belongs to, for grouping reports.
pub fn family_of(release: &Release) -> &'static str {
    release.family
}

/// True when the layout is one this bridge has a converter for. Every layout
/// in the database does; the function exists so that adding one without a
/// converter fails a test instead of silently producing a bad package.
pub fn layout_is_supported(layout: Layout) -> bool {
    matches!(
        layout,
        Layout::Legacy | Layout::Flattened | Layout::SingularFolders
            | Layout::ItemDefinitions | Layout::ModernHorizon
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jar::read_bytes;

    fn jar_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let entries: Vec<hb_zip::OutEntry> = files
            .iter()
            .map(|(name, bytes)| hb_zip::OutEntry::new(*name, bytes.clone()))
            .collect();
        hb_zip::build(&entries).unwrap()
    }

    fn class_file(major: u16) -> Vec<u8> {
        let mut pool: Vec<Vec<u8>> = Vec::new();
        let utf8 = |pool: &mut Vec<Vec<u8>>, text: &str| -> u16 {
            let mut entry = vec![1u8];
            entry.extend_from_slice(&(text.len() as u16).to_be_bytes());
            entry.extend_from_slice(text.as_bytes());
            pool.push(entry);
            pool.len() as u16
        };
        let name_at = utf8(&mut pool, "com/example/Mod");
        let super_at = utf8(&mut pool, "java/lang/Object");
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
        out.extend_from_slice(&major.to_be_bytes());
        out.extend_from_slice(&((pool.len() + 1) as u16).to_be_bytes());
        for entry in &pool {
            out.extend_from_slice(entry);
        }
        out.extend_from_slice(&0x0021u16.to_be_bytes());
        out.extend_from_slice(&this_at.to_be_bytes());
        out.extend_from_slice(&parent_at.to_be_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    const FABRIC_1211: &str =
        r#"{"schemaVersion":1,"id":"example","version":"1.0.0","depends":{"minecraft":"1.21.1"}}"#;

    #[test]
    fn reads_a_stated_fabric_version() {
        let bytes = jar_bytes(&[
            ("fabric.mod.json", FABRIC_1211.as_bytes().to_vec()),
            ("com/example/Mod.class", class_file(61)),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        assert_eq!(detection.loader, Some(Loader::Fabric));
        assert_eq!(detection.version.map(|v| v.id), Some("1.21.1"));
        assert_eq!(detection.confidence, Confidence::Stated);
        assert_eq!(detection.class_major, Some(61));
        assert!(detection.has_java());
        assert_eq!(detection.loader_label(), "fabric");
    }

    #[test]
    fn a_range_is_narrowed_not_stated() {
        let bytes = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"example","version":"1","depends":{"minecraft":">=1.21 <1.21.5"}}"#
                .to_vec(),
        )]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        assert!(detection.version.is_none());
        assert_eq!(detection.confidence, Confidence::Narrowed);
        assert!(detection.versions.len() > 1, "{:?}", detection.versions);
        assert!(detection.versions.iter().all(|v| v.id.starts_with("1.21")));
        assert!(detection
            .concerns
            .iter()
            .any(|(code, _)| code == "detect.version.range"));
    }

    #[test]
    fn a_class_only_jar_does_not_run() {
        let bytes = jar_bytes(&[
            ("fabric.mod.json", FABRIC_1211.as_bytes().to_vec()),
            ("com/example/Mod.class", class_file(61)),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        let (verdict, why) = content_verdict(&jar, &detection);
        assert_eq!(verdict, Verdict::WontRun);
        assert!(why.contains("does not run Java"), "{why}");
    }

    #[test]
    fn content_plus_code_is_partly() {
        let bytes = jar_bytes(&[
            ("fabric.mod.json", FABRIC_1211.as_bytes().to_vec()),
            ("com/example/Mod.class", class_file(61)),
            (
                "assets/example/lang/en_us.json",
                br#"{"a":"b"}"#.to_vec(),
            ),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        let (verdict, _) = content_verdict(&jar, &detection);
        assert_eq!(verdict, Verdict::Partly);
    }

    #[test]
    fn a_content_only_jar_works() {
        let bytes = jar_bytes(&[
            ("fabric.mod.json", FABRIC_1211.as_bytes().to_vec()),
            (
                "assets/example/lang/en_us.json",
                br#"{"a":"b"}"#.to_vec(),
            ),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        let (verdict, _) = content_verdict(&jar, &detection);
        assert_eq!(verdict, Verdict::Works);
    }

    #[test]
    fn an_old_class_version_is_flagged_and_banded() {
        let bytes = jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"example","version":"1"}"#.to_vec(),
            ),
            ("com/example/Mod.class", class_file(50)),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        assert_eq!(detection.confidence, Confidence::Guessed);
        assert!(detection
            .versions
            .iter()
            .any(|r| r.id == "1.8.9"));
        assert!(detection
            .concerns
            .iter()
            .any(|(code, _)| code == "detect.version.guessed"));
    }

    #[test]
    fn nothing_at_all_is_reported_as_nothing() {
        let bytes = jar_bytes(&[("readme.txt", b"hello".to_vec())]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        let (verdict, _) = content_verdict(&jar, &detection);
        assert_eq!(verdict, Verdict::Nothing);
        assert!(detection
            .concerns
            .iter()
            .any(|(code, _)| code == "detect.unrecognised"));
    }

    #[test]
    fn layout_alone_suggests_the_oldest_era() {
        let bytes = jar_bytes(&[(
            "assets/old/textures/blocks/stone.png",
            b"not checked here".to_vec(),
        )]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let detection = detect(&jar);
        assert_eq!(detection.version.map(|v| v.id), Some("1.8.9"));
        assert_eq!(detection.confidence, Confidence::Guessed);
    }

    #[test]
    fn every_layout_has_a_converter() {
        for release in hb_versions::targets() {
            assert!(
                layout_is_supported(release.layout),
                "{} has no converter",
                release.id
            );
        }
    }
}
