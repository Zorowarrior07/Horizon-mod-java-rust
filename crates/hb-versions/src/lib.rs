//! The version compatibility database.
//!
//! One place that knows, for every Minecraft Java release the bridge targets:
//!
//! * its resource pack format and data pack format (the numbers that decide
//!   whether a pack loads at all);
//! * which resource *layout* it uses (pre-flattening plural folders, 1.13+
//!   singular folders, 1.21's folder renames, 26.x item model definitions);
//! * which metadata file a mod of that version carries;
//! * whether it is a supported target, a detection-only entry, or unsupported.
//!
//! Everything here is data, cited in `docs/COMPATIBILITY.md`. Nothing is
//! inferred at runtime: a version missing from this table is reported as
//! unknown rather than guessed at.
//!
//! Formats are `(major, minor)`. Before 1.21.9 there was only a major number;
//! from 1.21.9 and 26.x on, a minor part exists and rises without breaking
//! packs, so `69` and `69.0` are the same format and compare equal.

use hb_json::Value;

/// A pack format: `major.minor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackFormat {
    pub major: u32,
    pub minor: u32,
}

impl PackFormat {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// A pre-1.21.9 single-number format.
    pub const fn major_only(major: u32) -> Self {
        Self { major, minor: 0 }
    }

    /// Accepts `48`, `"48"`, `69.0` (as JSON text `69.0`), or a `[major,
    /// minor]` array, which is how `min_format` and `max_format` may be
    /// written from 1.21.9.
    pub fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Num(n) => {
                let major = n.as_i64()?;
                // `69.0` arrives as a floating literal; its fractional part
                // must be zero, and the minor part is then not expressible
                // this way (the game uses arrays for that).
                let text = n.text();
                if let Some((whole, fraction)) = text.split_once('.') {
                    if fraction.chars().all(|c| c == '0') {
                        return Some(Self::major_only(whole.parse().ok()?));
                    }
                    return None;
                }
                u32::try_from(major).ok().map(Self::major_only)
            }
            Value::Arr(items) if items.len() == 2 => Some(Self::new(
                u32::try_from(items[0].as_i64()?).ok()?,
                u32::try_from(items[1].as_i64()?).ok()?,
            )),
            _ => None,
        }
    }

    /// How the game writes it in `pack.mcmeta` for the given era.
    pub fn to_json(self, major_only_style: bool) -> Value {
        if major_only_style {
            Value::int(self.major)
        } else if self.minor == 0 {
            Value::int(self.major)
        } else {
            Value::arr([Value::int(self.major), Value::int(self.minor)])
        }
    }

    pub fn to_string_short(self) -> String {
        if self.minor == 0 {
            format!("{}", self.major)
        } else {
            format!("{}.{}", self.major, self.minor)
        }
    }

    /// Parses `48`, `69.0`, `[69, 0]`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            let (major, minor) = inner.split_once(',')?;
            return Some(Self::new(major.trim().parse().ok()?, minor.trim().parse().ok()?));
        }
        match text.split_once('.') {
            Some((major, minor)) => {
                Some(Self::new(major.parse().ok()?, minor.parse().ok()?))
            }
            None => text.parse().ok().map(Self::major_only),
        }
    }
}

impl std::fmt::Display for PackFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_short())
    }
}

/// The layout of `assets/` and `data/` a release reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 1.6–1.12: `textures/blocks/`, `textures/items/`, `models/block/`,
    /// `lang/en_US.lang`, `data/<ns>/recipes/`, and numeric block ids in
    /// models rather than names.
    Legacy,
    /// 1.13–1.20.6: flattening done, `textures/block/`, `data/<ns>/recipes/`
    /// (plural), `assets/<ns>/models/item/*.json`.
    Flattened,
    /// 1.21–1.21.3: folders renamed to the singular (`recipe/`, `loot_table/`,
    /// `tags/block/`), item models still in `models/item/`.
    SingularFolders,
    /// 1.21.4+: item model *definitions* in `assets/<ns>/items/`, with
    /// `models/item/` demoted to a parent the definition points at.
    ItemDefinitions,
    /// 26.x: item definitions plus split atlases (`blocks` and `items`),
    /// 26.3's palette move, and the `iron_chain` rename.
    ModernHorizon,
}

impl Layout {
    pub fn label(self) -> &'static str {
        match self {
            Layout::Legacy => "legacy (pre-flattening)",
            Layout::Flattened => "flattened (1.13-1.20.6)",
            Layout::SingularFolders => "singular data folders (1.21-1.21.3)",
            Layout::ItemDefinitions => "item model definitions (1.21.4+)",
            Layout::ModernHorizon => "26.x with split atlases",
        }
    }
}

/// The mod metadata a release's toolchain writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModMetadata {
    /// 1.8.9 Forge: `mcmod.info` (JSON array).
    ForgeLegacy,
    /// 1.13–1.20.4 Forge: `META-INF/mods.toml` with a `[[mods]]` table.
    ForgeToml,
    /// 1.20.5+ NeoForge: `META-INF/neoforge.mods.toml`.
    NeoForgeToml,
    /// Fabric, all versions: `fabric.mod.json`.
    Fabric,
    /// Quilt, all versions: `quilt.mod.json`.
    Quilt,
}

impl ModMetadata {
    pub fn file(self) -> &'static str {
        match self {
            ModMetadata::ForgeLegacy => "mcmod.info",
            ModMetadata::ForgeToml => "META-INF/mods.toml",
            ModMetadata::NeoForgeToml => "META-INF/neoforge.mods.toml",
            ModMetadata::Fabric => "fabric.mod.json",
            ModMetadata::Quilt => "quilt.mod.json",
        }
    }

    pub fn loader(self) -> &'static str {
        match self {
            ModMetadata::ForgeLegacy | ModMetadata::ForgeToml => "forge",
            ModMetadata::NeoForgeToml => "neoforge",
            ModMetadata::Fabric => "fabric",
            ModMetadata::Quilt => "quilt",
        }
    }
}

/// One Minecraft Java release the bridge knows about.
#[derive(Debug, Clone)]
pub struct Release {
    /// `"1.21.4"`, `"26.3"`.
    pub id: &'static str,
    /// Which target family it belongs to: `"1.8.9"`, `"1.21.x"`, `"26.x"`.
    pub family: &'static str,
    pub resource_format: PackFormat,
    pub data_format: PackFormat,
    pub layout: Layout,
    /// Release date, `YYYY-MM-DD`, for reports.
    pub released: &'static str,
    /// Whether the bridge can produce packages for it.
    pub target: bool,
    /// Mod metadata files its toolchains write, newest first.
    pub metadata: &'static [ModMetadata],
    /// Whether the game reads `min_format`/`max_format` (1.21.9+).
    pub min_max_format: bool,
}

impl Release {
    /// True when the two releases' formats make a pack portable without
    /// conversion.
    pub fn same_format_as(&self, other: &Release) -> bool {
        self.resource_format == other.resource_format
            && self.data_format == other.data_format
            && self.layout == other.layout
    }
}

/// Every release the bridge knows. Detection covers more versions than
/// packaging does; `target` distinguishes them.
pub const RELEASES: &[Release] = &[
    Release {
        id: "1.8.9",
        family: "1.8.9",
        resource_format: PackFormat::major_only(1),
        data_format: PackFormat::major_only(1),
        layout: Layout::Legacy,
        released: "2015-12-09",
        target: true,
        metadata: &[ModMetadata::ForgeLegacy],
        min_max_format: false,
    },
    Release {
        id: "1.12.2",
        family: "1.8.9",
        resource_format: PackFormat::major_only(3),
        data_format: PackFormat::major_only(1),
        layout: Layout::Legacy,
        released: "2017-09-18",
        target: false,
        metadata: &[ModMetadata::ForgeLegacy],
        min_max_format: false,
    },
    Release {
        id: "1.20.1",
        family: "1.21.x",
        resource_format: PackFormat::major_only(15),
        data_format: PackFormat::major_only(15),
        layout: Layout::Flattened,
        released: "2023-06-12",
        target: false,
        metadata: &[ModMetadata::ForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21",
        family: "1.21.x",
        resource_format: PackFormat::major_only(34),
        data_format: PackFormat::major_only(48),
        layout: Layout::SingularFolders,
        released: "2024-06-13",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::ForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.1",
        family: "1.21.x",
        resource_format: PackFormat::major_only(34),
        data_format: PackFormat::major_only(48),
        layout: Layout::SingularFolders,
        released: "2024-08-08",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::ForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.2",
        family: "1.21.x",
        resource_format: PackFormat::major_only(42),
        data_format: PackFormat::major_only(57),
        layout: Layout::SingularFolders,
        released: "2024-10-22",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.3",
        family: "1.21.x",
        resource_format: PackFormat::major_only(42),
        data_format: PackFormat::major_only(57),
        layout: Layout::SingularFolders,
        released: "2024-10-23",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.4",
        family: "1.21.x",
        resource_format: PackFormat::major_only(46),
        data_format: PackFormat::major_only(61),
        layout: Layout::ItemDefinitions,
        released: "2024-12-03",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.5",
        family: "1.21.x",
        resource_format: PackFormat::major_only(55),
        data_format: PackFormat::major_only(71),
        layout: Layout::ItemDefinitions,
        released: "2025-03-25",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.6",
        family: "1.21.x",
        resource_format: PackFormat::major_only(63),
        data_format: PackFormat::major_only(80),
        layout: Layout::ItemDefinitions,
        released: "2025-06-17",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.7",
        family: "1.21.x",
        resource_format: PackFormat::major_only(64),
        data_format: PackFormat::major_only(81),
        layout: Layout::ItemDefinitions,
        released: "2025-06-30",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.8",
        family: "1.21.x",
        resource_format: PackFormat::major_only(64),
        data_format: PackFormat::major_only(81),
        layout: Layout::ItemDefinitions,
        released: "2025-07-17",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: false,
    },
    Release {
        id: "1.21.9",
        family: "1.21.x",
        resource_format: PackFormat::new(69, 0),
        data_format: PackFormat::new(88, 0),
        layout: Layout::ItemDefinitions,
        released: "2025-09-30",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "1.21.10",
        family: "1.21.x",
        resource_format: PackFormat::new(69, 0),
        data_format: PackFormat::new(88, 0),
        layout: Layout::ItemDefinitions,
        released: "2025-10-09",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "1.21.11",
        family: "1.21.x",
        resource_format: PackFormat::new(75, 0),
        data_format: PackFormat::new(94, 1),
        layout: Layout::ItemDefinitions,
        released: "2025-11-25",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "26.1",
        family: "26.x",
        resource_format: PackFormat::new(84, 0),
        data_format: PackFormat::new(101, 1),
        layout: Layout::ModernHorizon,
        released: "2026-02-10",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "26.1.1",
        family: "26.x",
        resource_format: PackFormat::new(84, 0),
        data_format: PackFormat::new(101, 1),
        layout: Layout::ModernHorizon,
        released: "2026-03-03",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "26.1.2",
        family: "26.x",
        resource_format: PackFormat::new(84, 0),
        data_format: PackFormat::new(101, 1),
        layout: Layout::ModernHorizon,
        released: "2026-04-07",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "26.2",
        family: "26.x",
        resource_format: PackFormat::new(88, 0),
        data_format: PackFormat::new(107, 1),
        layout: Layout::ModernHorizon,
        released: "2026-06-16",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
    Release {
        id: "26.3",
        family: "26.x",
        resource_format: PackFormat::new(97, 1),
        data_format: PackFormat::new(121, 0),
        layout: Layout::ModernHorizon,
        released: "2026-09-22",
        target: true,
        metadata: &[ModMetadata::NeoForgeToml, ModMetadata::Fabric],
        min_max_format: true,
    },
];

/// Look up a release by its exact id.
pub fn release(id: &str) -> Option<&'static Release> {
    RELEASES.iter().find(|r| r.id == id)
}

/// Every packaging target, newest first.
pub fn targets() -> impl Iterator<Item = &'static Release> {
    RELEASES.iter().filter(|r| r.target)
}

/// The Horizon content API's format: what a `.hmod` is validated against.
pub fn horizon_target() -> &'static Release {
    release("26.3").expect("26.3 is in the table")
}

/// The release a pack format pair belongs to, newest match first.
pub fn release_for_formats(resource: PackFormat, data: PackFormat) -> Option<&'static Release> {
    RELEASES
        .iter()
        .rev()
        .find(|r| r.resource_format == resource && r.data_format == data)
        .or_else(|| {
            // A pack that declares only a resource format (a resource pack
            // has no data format of its own).
            RELEASES
                .iter()
                .rev()
                .find(|r| r.resource_format == resource)
        })
}

/// Every release sharing a format pair, oldest first. A pack made for 1.21
/// works on 1.21.1 too, and a report should say so rather than pick one.
pub fn releases_for_formats(resource: PackFormat, data: PackFormat) -> Vec<&'static Release> {
    RELEASES
        .iter()
        .filter(|r| r.resource_format == resource && r.data_format == data)
        .collect()
}

/// Parses a version string such as `"1.21.4"`, `"v1.21.4"`, `"1.21"`,
/// `"26.3"`. Returns the release when the database knows it.
pub fn parse_version(text: &str) -> Option<&'static Release> {
    let cleaned = text
        .trim()
        .trim_start_matches(['v', 'V'])
        .trim();
    // Tolerate a loader-decorated version such as `1.21.1-beta.3` or
    // `>=1.21 <1.22` by taking the leading dotted number.
    let digits: String = cleaned
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let digits = digits.trim_end_matches('.');
    if digits.is_empty() {
        return None;
    }
    release(digits)
}

/// A version requirement expressed the way mod metadata writes it:
/// `[1.21,1.22)`, `>=1.21 <1.22`, `1.21.1`, `*`, `>=1.21`.
///
/// Only the shapes real mod files use are supported; anything else returns
/// [`Requirement::Unknown`] so the report can say "could not parse" instead of
/// pretending the mod definitely targets something.
#[derive(Debug, Clone, PartialEq)]
pub enum Requirement {
    Any,
    Exactly(String),
    /// Inclusive lower bound, exclusive upper bound, as dotted versions.
    Range { low: String, low_inclusive: bool, high: Option<String>, high_inclusive: bool },
    Unknown(String),
}

impl Requirement {
    pub fn parse(text: &str) -> Requirement {
        let text = text.trim();
        if text.is_empty() || text == "*" {
            return Requirement::Any;
        }
        if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            if let Some((low, high)) = inner.split_once(',') {
                return Requirement::Range {
                    low: low.trim().to_owned(),
                    low_inclusive: true,
                    high: Some(high.trim().to_owned()),
                    high_inclusive: true,
                };
            }
        }
        if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(')')) {
            if let Some((low, high)) = inner.split_once(',') {
                return Requirement::Range {
                    low: low.trim().to_owned(),
                    low_inclusive: true,
                    high: Some(high.trim().to_owned()),
                    high_inclusive: false,
                };
            }
        }
        if let Some(rest) = text.strip_prefix(">=") {
            let rest = rest.trim();
            if let Some((low, high)) = rest.split_once('<') {
                return Requirement::Range {
                    low: low.trim().to_owned(),
                    low_inclusive: true,
                    high: Some(high.trim().trim_end_matches('=').trim().to_owned()),
                    high_inclusive: high.trim().ends_with('='),
                };
            }
            return Requirement::Range {
                low: rest.to_owned(),
                low_inclusive: true,
                high: None,
                high_inclusive: false,
            };
        }
        if text.contains(',') || text.contains(' ') {
            return Requirement::Unknown(text.to_owned());
        }
        Requirement::Exactly(text.trim_start_matches(['v', 'V']).to_owned())
    }

    /// Whether `version` satisfies the requirement. Unknown versions in the
    /// database compare by numeric segments.
    pub fn accepts(&self, version: &str) -> bool {
        match self {
            Requirement::Any => true,
            Requirement::Exactly(want) => version_key(version) == version_key(want),
            Requirement::Range {
                low,
                low_inclusive,
                high,
                high_inclusive,
            } => {
                let v = version_key(version);
                if !low.is_empty() {
                    let l = version_key(low);
                    if v < l || (!low_inclusive && v == l) {
                        return false;
                    }
                }
                if let Some(high) = high {
                    if !high.is_empty() {
                        let h = version_key(high);
                        if v > h || (!high_inclusive && v == h) {
                            return false;
                        }
                    }
                }
                true
            }
            Requirement::Unknown(_) => false,
        }
    }
}

/// A comparable key for a Minecraft version: zero-padded numeric segments, so
/// `1.21.10 > 1.21.9` and `26.3 > 26.1.2`.
pub fn version_key(text: &str) -> [u16; 4] {
    let mut key = [0u16; 4];
    for (slot, part) in text
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .take(4)
        .enumerate()
    {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        key[slot] = digits.parse().unwrap_or(0);
    }
    key
}

/// The database as JSON, for `hb-cli matrix --json` and for tooling outside
/// this repository.
pub fn database_json() -> Value {
    let entries: Vec<Value> = RELEASES
        .iter()
        .map(|r| {
            Value::obj([
                ("id", Value::str(r.id)),
                ("family", Value::str(r.family)),
                ("resource_format", Value::str(r.resource_format.to_string_short())),
                ("data_format", Value::str(r.data_format.to_string_short())),
                ("layout", Value::str(r.layout.label())),
                ("released", Value::str(r.released)),
                ("target", Value::Bool(r.target)),
                (
                    "metadata",
                    Value::arr(r.metadata.iter().map(|m| Value::str(m.file()))),
                ),
                ("min_max_format", Value::Bool(r.min_max_format)),
            ])
        })
        .collect();
    Value::obj([
        (
            "schema",
            Value::str("zoro-horizon-bridge/version-database@1"),
        ),
        (
            "horizon_content_api",
            Value::obj([
                ("api", Value::str("0.2")),
                (
                    "game",
                    Value::str(horizon_target().id),
                ),
                (
                    "resource_format",
                    Value::str(horizon_target().resource_format.to_string_short()),
                ),
                (
                    "data_format",
                    Value::str(horizon_target().data_format.to_string_short()),
                ),
            ]),
        ),
        ("releases", Value::Arr(entries)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_parse_and_compare() {
        assert_eq!(PackFormat::parse("48"), Some(PackFormat::new(48, 0)));
        assert_eq!(PackFormat::parse("69.0"), Some(PackFormat::new(69, 0)));
        assert_eq!(PackFormat::parse("[97, 1]"), Some(PackFormat::new(97, 1)));
        assert_eq!(PackFormat::parse("x"), None);
        assert!(PackFormat::new(97, 1) > PackFormat::new(88, 0));
        // Minor versions compare as a pair, so 69.0 == 69.
        assert_eq!(PackFormat::major_only(69), PackFormat::new(69, 0));
    }

    #[test]
    fn every_target_has_distinct_and_ordered_formats() {
        for release in RELEASES {
            assert!(release.resource_format.major > 0, "{}", release.id);
            assert!(release.data_format.major > 0, "{}", release.id);
        }
        // Resource formats rise monotonically across the table.
        let mut previous = PackFormat::new(0, 0);
        for release in RELEASES {
            assert!(
                release.resource_format >= previous,
                "{} went backwards",
                release.id
            );
            previous = release.resource_format;
        }
        // And no two releases share an id.
        for (i, a) in RELEASES.iter().enumerate() {
            for b in &RELEASES[i + 1..] {
                assert_ne!(a.id, b.id);
            }
        }
    }

    #[test]
    fn covers_every_1_21_minor_and_26_release() {
        for id in [
            "1.21", "1.21.1", "1.21.2", "1.21.3", "1.21.4", "1.21.5", "1.21.6", "1.21.7",
            "1.21.8", "1.21.9", "1.21.10", "1.21.11",
        ] {
            assert!(release(id).is_some(), "1.21 series is missing {id}");
            assert!(release(id).unwrap().target);
        }
        for id in ["26.1", "26.1.1", "26.1.2", "26.2", "26.3"] {
            assert!(release(id).is_some(), "26.x series is missing {id}");
            assert!(release(id).unwrap().target);
        }
        assert!(release("1.8.9").unwrap().target);
    }

    #[test]
    fn parses_versions_as_the_game_reports_them() {
        assert_eq!(parse_version("1.21.4").unwrap().id, "1.21.4");
        assert_eq!(parse_version("v1.21.4").unwrap().id, "1.21.4");
        assert_eq!(parse_version("26.3").unwrap().id, "26.3");
        assert_eq!(parse_version("1.21.4-beta.2").unwrap().id, "1.21.4");
        assert!(parse_version("1.7.10").is_none());
        assert!(parse_version("garbage").is_none());
    }

    #[test]
    fn version_keys_order_numerically() {
        assert!(version_key("1.21.10") > version_key("1.21.9"));
        assert!(version_key("26.3") > version_key("26.1.2"));
        assert!(version_key("1.8.9") < version_key("1.21"));
        assert_eq!(version_key("v1.21"), version_key("1.21"));
    }

    #[test]
    fn requirements_read_mod_metadata_shapes() {
        let range = Requirement::parse("[1.21,1.22)");
        assert!(range.accepts("1.21"));
        assert!(range.accepts("1.21.4"));
        assert!(!range.accepts("1.22"));
        assert!(!range.accepts("1.20.1"));

        let at_least = Requirement::parse(">=1.21 <1.22");
        assert!(at_least.accepts("1.21.9"));
        assert!(!at_least.accepts("1.22"));

        let exact = Requirement::parse("1.21.1");
        assert!(exact.accepts("1.21.1"));
        assert!(!exact.accepts("1.21.2"));

        assert!(Requirement::parse("*").accepts("26.3"));
        assert!(Requirement::parse("").accepts("1.8.9"));
        // NeoForge writes `[1.21.1,1.22)` and Forge `[1.21.1,1.22)`; both work.
        assert!(Requirement::parse("[1.21.1,1.22)").accepts("1.21.4"));
        // Something we do not understand never silently matches.
        assert!(!Requirement::parse("^1.21 || ^1.22").accepts("1.21.4"));
    }

    #[test]
    fn finds_releases_from_declared_formats() {
        assert_eq!(
            release_for_formats(PackFormat::major_only(34), PackFormat::major_only(48))
                .unwrap()
                .id,
            // 1.21 and 1.21.1 share formats 34/48; the newest is reported and
            // the report lists the whole set, so no release is hidden.
            "1.21.1"
        );
        assert_eq!(
            release_for_formats(PackFormat::new(97, 1), PackFormat::new(121, 0))
                .unwrap()
                .id,
            "26.3"
        );
        assert_eq!(horizon_target().id, "26.3");
    }

    #[test]
    fn json_export_lists_every_release() {
        let json = database_json();
        let releases = json.get("releases").unwrap().as_arr().unwrap();
        assert_eq!(releases.len(), RELEASES.len());
        let first = &releases[0];
        assert_eq!(first.get_str("id"), Some("1.8.9"));
        assert_eq!(
            json.get("horizon_content_api").unwrap().get_str("api"),
            Some("0.2")
        );
    }
}
