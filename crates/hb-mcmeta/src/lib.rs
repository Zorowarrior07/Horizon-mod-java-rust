//! Reads a Java mod's identity and requirements from its own metadata.
//!
//! Order of precedence, matching what each loader actually installs:
//!
//! 1. `META-INF/neoforge.mods.toml` (NeoForge, 1.20.5+)
//! 2. `META-INF/mods.toml` (Forge, 1.13+)
//! 3. `fabric.mod.json` (Fabric)
//! 4. `quilt.mod.json` (Quilt; rarely seen, read because a jar may be either)
//! 5. `mcmod.info` (legacy Forge, through 1.12.2 — the 1.8.9 target)
//!
//! Identity comes from the metadata, never from the filename. A jar whose
//! metadata is missing or unreadable is reported as [`Metadata::Unknown`] with
//! the reason, so the caller can refuse it rather than invent an id.

pub mod toml;

use hb_json::{Object, Value};
use std::collections::BTreeMap;

/// Which loader's metadata was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loader {
    NeoForge,
    Forge,
    Fabric,
    Quilt,
    LegacyForge,
}

impl Loader {
    pub fn id(&self) -> &'static str {
        match self {
            Loader::NeoForge => "neoforge",
            Loader::Forge => "forge",
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::LegacyForge => "forge (legacy)",
        }
    }

    /// Whether this loader's jars are single-jar multi-version (`fabric`) or
    /// one version per jar (`forge`, `neoforge`), which decides how strictly a
    /// declared game version can be trusted.
    pub fn multi_version_jar(&self) -> bool {
        matches!(self, Loader::Fabric | Loader::Quilt)
    }
}

/// A declared dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    /// The requirement text as written, e.g. `[1.21.1,1.22)`.
    pub requirement: String,
    pub required: bool,
    /// `BOTH`, `CLIENT`, `SERVER` when the metadata says.
    pub side: Option<String>,
}

/// A mod's identity plus what it says it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ModInfo {
    pub loader: Loader,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub homepage: Option<String>,
    pub issues: Option<String>,
    pub dependencies: Vec<Dependency>,
    /// Minecraft version requirements, in the order declared.
    pub minecraft: Vec<Dependency>,
    /// Entry point classes: main, client, server.
    pub entry_points: Vec<EntryPoint>,
    /// Mixin configuration file names the metadata lists.
    pub mixin_configs: Vec<String>,
    /// Access wideners (Fabric) or access transformers (Forge) the jar needs.
    pub access_files: Vec<String>,
    /// Fields the metadata wrote that we read but do not act on, for the
    /// report.
    pub extras: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPoint {
    pub kind: String,
    pub class: String,
}

/// The result of reading a jar's metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum Metadata {
    Found(ModInfo),
    /// No metadata file at all.
    Absent { looked_for: Vec<&'static str> },
    /// Metadata present but unreadable.
    Invalid { file: String, reason: String },
}

impl Metadata {
    pub fn info(&self) -> Option<&ModInfo> {
        match self {
            Metadata::Found(info) => Some(info),
            _ => None,
        }
    }
}

/// The files searched, in precedence order.
pub const METADATA_FILES: &[&str] = &[
    "META-INF/neoforge.mods.toml",
    "META-INF/mods.toml",
    "fabric.mod.json",
    "quilt.mod.json",
    "mcmod.info",
];

/// Reads metadata from a file map (`path -> bytes`), as a jar reader produces.
pub fn read<F>(mut get: F) -> Metadata
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    if get("META-INF/neoforge.mods.toml").is_some() {
        return read_forge_toml("META-INF/neoforge.mods.toml", Loader::NeoForge, &mut get);
    }
    if get("META-INF/mods.toml").is_some() {
        return read_forge_toml("META-INF/mods.toml", Loader::Forge, &mut get);
    }
    if get("fabric.mod.json").is_some() {
        return read_fabric("fabric.mod.json", Loader::Fabric, &mut get);
    }
    if get("quilt.mod.json").is_some() {
        return read_fabric("quilt.mod.json", Loader::Quilt, &mut get);
    }
    if get("mcmod.info").is_some() {
        return read_mcmod_info(&mut get);
    }
    // A `META-INF/MANIFEST.MF` alone (a plain library jar) is not a mod.
    Metadata::Absent {
        looked_for: METADATA_FILES.to_vec(),
    }
}

fn text(bytes: &[u8]) -> String {
    // Metadata is UTF-8; a stray BOM or invalid byte must not lose the file.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Resolves `authors` in either the Fabric array form or Forge's
/// comma-separated string form.
fn authors(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Arr(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_owned()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::Str(s)) => s
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn string_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Arr(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => hb_json::to_string(other),
        None => String::new(),
    }
}

fn read_forge_toml<F>(file: &str, loader: Loader, get: &mut F) -> Metadata
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    let raw = get(file).expect("checked by caller");
    let document = match toml::parse(&text(&raw)) {
        Ok(v) => v,
        Err(e) => {
            return Metadata::Invalid {
                file: file.to_owned(),
                reason: e.to_string(),
            }
        }
    };
    let Some(mods) = document.get("mods") else {
        return Metadata::Invalid {
            file: file.to_owned(),
            reason: "no [[mods]] table: this is not a mod jar, or the table is spelled differently"
                .into(),
        };
    };
    // `[[mods]]` parses as an array; a single `[mods]` table also appears.
    let first = match mods {
        Value::Arr(items) => items.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    if !first.is_obj() {
        return Metadata::Invalid {
            file: file.to_owned(),
            reason: "[[mods]] contains no table".into(),
        };
    }
    let id = first.get_str("modId").unwrap_or_default().trim().to_owned();
    if id.is_empty() {
        return Metadata::Invalid {
            file: file.to_owned(),
            reason: "[[mods]] has no modId".into(),
        };
    }
    let mut info = ModInfo {
        loader,
        id,
        name: first
            .get_str("displayName")
            .unwrap_or_default()
            .trim()
            .to_owned(),
        version: first.get_str("version").unwrap_or_default().trim().to_owned(),
        description: string_of(first.get("description")),
        authors: authors(first.get("authors")),
        license: first.get_str("license").unwrap_or_default().to_owned(),
        homepage: first.get_str("displayURL").map(|s| s.to_owned()),
        issues: first.get_str("issueTrackerURL").map(|s| s.to_owned()),
        dependencies: Vec::new(),
        minecraft: Vec::new(),
        entry_points: Vec::new(),
        mixin_configs: Vec::new(),
        access_files: Vec::new(),
        extras: BTreeMap::new(),
    };
    if info.name.is_empty() {
        info.name = info.id.clone();
    }
    if let Some(value) = first.get_str("modLoader") {
        info.extras.insert("modLoader".into(), value.to_owned());
    }
    // Dependencies live under `[dependencies.<modid>]`, either a table or an
    // array of tables. `minecraft` and the loader are separated out.
    if let Some(deps) = document.get("dependencies") {
        for (owner, value) in deps.as_obj().map(|o| o.iter().collect::<Vec<_>>()).unwrap_or_default()
        {
            let entries: Vec<&Value> = match value {
                Value::Arr(items) => items.iter().collect(),
                other => vec![other],
            };
            for entry in entries {
                let dep_id = entry.get_str("modId").unwrap_or(owner).trim().to_owned();
                let requirement = entry
                    .get_str("versionRange")
                    .unwrap_or("*")
                    .trim()
                    .to_owned();
                let required = entry
                    .get("mandatory")
                    .and_then(Value::as_bool)
                    .or_else(|| {
                        entry
                            .get_str("type")
                            .map(|t| !t.eq_ignore_ascii_case("optional"))
                    })
                    .unwrap_or(true);
                let dependency = Dependency {
                    id: dep_id.clone(),
                    requirement,
                    required,
                    side: entry.get_str("side").map(|s| s.to_owned()),
                };
                if dep_id == "minecraft" {
                    info.minecraft.push(dependency);
                } else if !is_loader_id(&dep_id) {
                    info.dependencies.push(dependency);
                }
            }
        }
    }
    read_common_files(&mut info, get);
    Metadata::Found(info)
}

fn is_loader_id(id: &str) -> bool {
    matches!(
        id,
        "neoforge" | "forge" | "fml" | "javafml" | "lowcodefml" | "minecraft" | "java"
    )
}

/// Reads mixin configs and access files that every loader declares in its own
/// way; the common cases are listed in each metadata file, and the rest are
/// found by name convention in [`crate::METADATA_FILES`] consumers.
fn read_common_files<F>(info: &mut ModInfo, get: &mut F)
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    if let Some(manifest) = get("META-INF/MANIFEST.MF") {
        for line in text(&manifest).lines() {
            if let Some(rest) = line.strip_prefix("MixinConfigs:") {
                for name in rest.split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        info.mixin_configs.push(name.to_owned());
                    }
                }
            }
        }
    }
}

fn read_fabric<F>(file: &str, loader: Loader, get: &mut F) -> Metadata
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    let raw = get(file).expect("checked by caller");
    let document = match hb_json::parse_bytes(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Metadata::Invalid {
                file: file.to_owned(),
                reason: e.to_string(),
            }
        }
    };
    // Quilt wraps its metadata one level down in a `quilt_loader` table.
    let root = if matches!(loader, Loader::Quilt) {
        match document.get("quilt_loader") {
            Some(inner) => inner.clone(),
            None => document.clone(),
        }
    } else {
        document.clone()
    };
    let id = root.get_str("id").unwrap_or_default().trim().to_owned();
    if id.is_empty() {
        return Metadata::Invalid {
            file: file.to_owned(),
            reason: "no \"id\" field".into(),
        };
    }
    let mut info = ModInfo {
        loader,
        id,
        name: root.get_str("name").unwrap_or_default().to_owned(),
        version: root.get_str("version").unwrap_or_default().to_owned(),
        description: string_of(root.get("description")),
        authors: authors(root.get("authors")),
        license: root.get_str("license").unwrap_or_default().to_owned(),
        homepage: root.get_str("contact").and_then(|_| None).or_else(|| {
            root.get("contact")
                .and_then(|c| c.get_str("homepage"))
                .map(|s| s.to_owned())
        }),
        issues: root
            .get("contact")
            .and_then(|c| c.get_str("issues"))
            .map(|s| s.to_owned()),
        dependencies: Vec::new(),
        minecraft: Vec::new(),
        entry_points: Vec::new(),
        mixin_configs: Vec::new(),
        access_files: Vec::new(),
        extras: BTreeMap::new(),
    };
    if info.name.is_empty() {
        info.name = info.id.clone();
    }
    if let Some(entries) = root.get("entrypoints").and_then(Value::as_obj) {
        for (kind, value) in entries.iter() {
            let list: Vec<&Value> = match value {
                Value::Arr(items) => items.iter().collect(),
                other => vec![other],
            };
            for entry in list {
                let class = match entry {
                    Value::Str(s) => s.clone(),
                    Value::Obj(_) => entry
                        .get_str("value")
                        .or_else(|| entry.get_str("adapter"))
                        .unwrap_or_default()
                        .to_owned(),
                    _ => continue,
                };
                if !class.is_empty() {
                    info.entry_points.push(EntryPoint {
                        kind: kind.to_owned(),
                        class: class.replace('.', "/"),
                    });
                }
            }
        }
    }
    if let Some(mixins) = root.get("mixins").and_then(Value::as_arr) {
        for mixin in mixins {
            match mixin {
                Value::Str(s) => info.mixin_configs.push(s.clone()),
                Value::Obj(_) => {
                    if let Some(file) = mixin.get_str("config") {
                        info.mixin_configs.push(file.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(aw) = root.get_str("accessWidener") {
        info.access_files.push(aw.to_owned());
    }
    if let Some(depends) = root.get("depends").and_then(Value::as_obj) {
        for (dep_id, requirement) in depends.iter() {
            let requirement = match requirement {
                Value::Str(s) => s.clone(),
                Value::Arr(items) => items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" || "),
                other => hb_json::to_string(other),
            };
            let dependency = Dependency {
                id: dep_id.to_owned(),
                requirement,
                required: true,
                side: None,
            };
            if dep_id == "minecraft" {
                info.minecraft.push(dependency);
            } else if dep_id != "java" && !dep_id.starts_with("fabric") && dep_id != "quilt_loader"
            {
                info.dependencies.push(dependency);
            }
        }
    }
    // Fabric's "breaks" and "conflicts" matter for a port: a mod that refuses
    // to load beside another cannot be merged into one package silently.
    if let Some(breaks) = root.get("breaks").and_then(Value::as_obj) {
        for (dep_id, requirement) in breaks.iter() {
            info.extras.insert(
                format!("breaks:{dep_id}"),
                match requirement {
                    Value::Str(s) => s.clone(),
                    other => hb_json::to_string(other),
                },
            );
        }
    }
    read_common_files(&mut info, get);
    Metadata::Found(info)
}

fn read_mcmod_info<F>(get: &mut F) -> Metadata
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    let file = "mcmod.info";
    let raw = get(file).expect("checked by caller");
    // Legacy Forge tolerated literal newlines inside JSON strings; Gson's
    // lenient parser accepted them. Escape them before parsing so those files
    // read instead of failing.
    let source = text(&raw);
    let repaired = escape_control_chars_in_strings(&source);
    let document = match hb_json::parse(&repaired) {
        Ok(v) => v,
        Err(e) => {
            return Metadata::Invalid {
                file: file.to_owned(),
                reason: e.to_string(),
            }
        }
    };
    let first = match document {
        Value::Arr(items) => items.first().cloned().unwrap_or(Value::Null),
        Value::Obj(_) => document.clone(),
        _ => Value::Null,
    };
    let id = first.get_str("modid").unwrap_or_default().trim().to_owned();
    if id.is_empty() {
        return Metadata::Invalid {
            file: file.to_owned(),
            reason: "no \"modid\" field".into(),
        };
    }
    let mut info = ModInfo {
        loader: Loader::LegacyForge,
        id,
        name: first.get_str("name").unwrap_or_default().to_owned(),
        version: first.get_str("version").unwrap_or_default().to_owned(),
        description: string_of(first.get("description")),
        authors: first
            .get("authorList")
            .and_then(Value::as_arr)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_else(|| authors(first.get("author"))),
        license: first.get_str("license").unwrap_or_default().to_owned(),
        homepage: first.get_str("url").map(|s| s.to_owned()),
        issues: None,
        dependencies: Vec::new(),
        minecraft: Vec::new(),
        entry_points: Vec::new(),
        mixin_configs: Vec::new(),
        access_files: Vec::new(),
        extras: BTreeMap::new(),
    };
    if info.name.is_empty() {
        info.name = info.id.clone();
    }
    // `mcmod.info` in 1.8.9 has no dependency block; the 1.8.9 target reads
    // requirements from `mcmod.info`'s `acceptedMinecraftVersions` when some
    // forks add it, and otherwise from the jar's own code.
    if let Some(accepted) = first.get_str("acceptedMinecraftVersions") {
        info.minecraft.push(Dependency {
            id: "minecraft".into(),
            requirement: accepted.to_owned(),
            required: true,
            side: None,
        });
    }
    if let Some(update) = first.get_str("updateJSON") {
        info.extras.insert("updateJSON".into(), update.to_owned());
    }
    read_common_files(&mut info, get);
    Metadata::Found(info)
}

/// Escapes raw control characters that appear inside JSON strings, which
/// legacy Forge's lenient Gson accepted and strict JSON forbids.
fn escape_control_chars_in_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in source.chars() {
        if escaped {
            escaped = false;
            out.push(c);
            continue;
        }
        match c {
            '\\' if in_string => {
                escaped = true;
                out.push(c);
            }
            '"' => {
                in_string = !in_string;
                out.push(c);
            }
            '\n' if in_string => out.push_str("\\n"),
            '\r' if in_string => out.push_str("\\r"),
            '\t' if in_string => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Parses a `*.mixins.json` file into a [`crate::MixinDocument`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixinDocument {
    pub file: String,
    pub package: Option<String>,
    pub mixins: Vec<String>,
    pub client: Vec<String>,
    pub server: Vec<String>,
    pub refmap: Option<String>,
    pub plugin: Option<String>,
    pub compatibility_level: Option<String>,
    pub required: bool,
    /// The document as read, for the report.
    pub raw: Option<Value>,
}

impl MixinDocument {
    pub fn classes(&self) -> Vec<String> {
        let prefix = self
            .package
            .as_deref()
            .map(|p| format!("{}/", p.replace('.', "/")));
        self.mixins
            .iter()
            .chain(&self.client)
            .chain(&self.server)
            .map(|name| match &prefix {
                Some(prefix) => format!("{prefix}{}", name.replace('.', "/")),
                None => name.replace('.', "/"),
            })
            .collect()
    }
}

/// Parses `*.mixins.json`. Returns the reason on malformed input so a jar can
/// be refused with a specific message.
pub fn parse_mixin_document(file: &str, bytes: &[u8]) -> Result<MixinDocument, String> {
    let value = hb_json::parse_bytes(bytes).map_err(|e| format!("{file}: {e}"))?;
    let names = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(Value::as_arr)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(MixinDocument {
        file: file.to_owned(),
        package: value.get_str("package").map(|s| s.to_owned()),
        mixins: names("mixins"),
        client: names("client"),
        server: names("server"),
        refmap: value.get_str("refmap").map(|s| s.to_owned()),
        plugin: value.get_str("plugin").map(|s| s.to_owned()),
        compatibility_level: value
            .get_str("compatibilityLevel")
            .map(|s| s.to_owned()),
        required: value
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        raw: Some(value),
    })
}

/// A pack's own `pack.mcmeta`, when the jar is also a resource or data pack.
#[derive(Debug, Clone, PartialEq)]
pub struct PackMeta {
    pub description: String,
    pub pack_format: Option<hb_versions::PackFormat>,
    pub supported_formats: Option<(u32, u32)>,
    pub min_format: Option<hb_versions::PackFormat>,
    pub max_format: Option<hb_versions::PackFormat>,
    pub filter: Vec<String>,
}

/// Parses `pack.mcmeta`. The `description` field may be a string or a JSON
/// text component object, both of which real packs use.
pub fn parse_pack_mcmeta(bytes: &[u8]) -> Result<PackMeta, String> {
    let value = hb_json::parse_bytes(bytes).map_err(|e| e.to_string())?;
    let pack = value
        .get("pack")
        .ok_or_else(|| "pack.mcmeta has no \"pack\" object".to_owned())?;
    let description = match pack.get("description") {
        Some(Value::Str(s)) => s.clone(),
        Some(other) => hb_json::to_string(other),
        None => String::new(),
    };
    let format_of = |key: &str| pack.get(key).and_then(hb_versions::PackFormat::from_json);
    let supported = match pack.get("supported_formats") {
        Some(Value::Arr(items)) if items.len() == 2 => Some((
            u32::try_from(items[0].as_i64().unwrap_or(0)).unwrap_or(0),
            u32::try_from(items[1].as_i64().unwrap_or(0)).unwrap_or(0),
        )),
        Some(Value::Obj(_)) => {
            let object = pack.get("supported_formats").and_then(Value::as_obj);
            Some((
                object
                    .and_then(|o| o.get("min_inclusive"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u32,
                object
                    .and_then(|o| o.get("max_inclusive"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u32,
            ))
        }
        _ => None,
    };
    let filter = value
        .get("filter")
        .and_then(|f| f.get("block"))
        .and_then(Value::as_arr)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.get_str("namespace").map(|s| s.to_owned()))
                .collect()
        })
        .unwrap_or_default();
    Ok(PackMeta {
        description,
        pack_format: format_of("pack_format"),
        supported_formats: supported,
        min_format: format_of("min_format"),
        max_format: format_of("max_format"),
        filter,
    })
}

/// Renders a `ModInfo` as JSON for reports.
pub fn info_json(info: &ModInfo) -> Value {
    Value::obj([
        ("loader", Value::str(info.loader.id())),
        ("id", Value::str(info.id.clone())),
        ("name", Value::str(info.name.clone())),
        ("version", Value::str(info.version.clone())),
        ("description", Value::str(info.description.clone())),
        (
            "authors",
            Value::arr(info.authors.iter().map(|a| Value::str(a.clone()))),
        ),
        ("license", Value::str(info.license.clone())),
        (
            "dependencies",
            Value::Arr(
                info.dependencies
                    .iter()
                    .map(|d| {
                        Value::obj([
                            ("id", Value::str(d.id.clone())),
                            ("requirement", Value::str(d.requirement.clone())),
                            ("required", Value::Bool(d.required)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "minecraft",
            Value::Arr(
                info.minecraft
                    .iter()
                    .map(|d| {
                        Value::obj([
                            ("requirement", Value::str(d.requirement.clone())),
                            ("required", Value::Bool(d.required)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "entry_points",
            Value::Arr(
                info.entry_points
                    .iter()
                    .map(|e| {
                        Value::obj([
                            ("kind", Value::str(e.kind.clone())),
                            ("class", Value::str(e.class.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "mixin_configs",
            Value::arr(info.mixin_configs.iter().map(|m| Value::str(m.clone()))),
        ),
        ("extras", Value::Obj(Object::new())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_map(files: &[(&str, &str)]) -> Metadata {
        read(|name| {
            files
                .iter()
                .find(|(path, _)| *path == name)
                .map(|(_, body)| body.as_bytes().to_vec())
        })
    }

    #[test]
    fn a_file_without_any_table_is_refused_rather_than_half_read() {
        let metadata = read_map(&[("META-INF/mods.toml", "")]);
        assert!(matches!(metadata, Metadata::Invalid { .. }));
    }

    #[test]
    fn prefers_neoforge_over_forge_and_fabric() {
        let metadata = read_map(&[
            (
                "META-INF/neoforge.mods.toml",
                "[[mods]]\nmodId = \"neo\"\nversion = \"2\"\n",
            ),
            ("META-INF/mods.toml", "[[mods]]\nmodId = \"forge\"\n"),
            ("fabric.mod.json", "{\"id\":\"fabric\"}"),
        ]);
        let info = metadata.info().unwrap();
        assert_eq!(info.id, "neo");
        assert_eq!(info.loader, Loader::NeoForge);
    }

    #[test]
    fn reads_fabric_metadata_fully() {
        let json = r#"{
          "schemaVersion": 1,
          "id": "example",
          "version": "1.2.3",
          "name": "Example",
          "description": "Does things.",
          "authors": ["A", "B"],
          "license": "MIT",
          "contact": {"homepage": "https://example.org", "issues": "https://example.org/i"},
          "entrypoints": {
            "main": ["com.example.ExampleMod"],
            "client": [{"value": "com.example.client.ExampleClient"}]
          },
          "mixins": ["example.mixins.json", {"config": "example.client.mixins.json", "environment": "client"}],
          "accessWidener": "example.accesswidener",
          "depends": {"minecraft": "~1.21.1", "fabricloader": ">=0.16", "fabric-api": "*", "example_core": ">=1.0"},
          "breaks": {"othermod": "<2.0"}
        }"#;
        let metadata = read_map(&[("fabric.mod.json", json)]);
        let info = metadata.info().unwrap();
        assert_eq!(info.loader, Loader::Fabric);
        assert_eq!(info.id, "example");
        assert_eq!(info.name, "Example");
        assert_eq!(info.authors, vec!["A", "B"]);
        assert_eq!(info.homepage.as_deref(), Some("https://example.org"));
        assert_eq!(info.minecraft[0].requirement, "~1.21.1");
        // The loader and fabric API are filtered out; a real mod dependency stays.
        assert_eq!(info.dependencies.len(), 1);
        assert_eq!(info.dependencies[0].id, "example_core");
        assert_eq!(info.entry_points.len(), 2);
        assert_eq!(info.entry_points[1].class, "com/example/client/ExampleClient");
        assert_eq!(info.mixin_configs.len(), 2);
        assert_eq!(info.access_files, vec!["example.accesswidener"]);
        assert!(info.extras.contains_key("breaks:othermod"));
    }

    #[test]
    fn reads_legacy_mcmod_info_with_literal_newlines() {
        // 1.8.9 jars routinely contain raw newlines inside JSON strings.
        let json = "[{\"modid\":\"old\",\"name\":\"Old\",\"description\":\"line one\nline two\",\
                    \"version\":\"1.0\",\"authorList\":[\"X\"],\"mcversion\":\"1.8.9\"}]";
        let metadata = read_map(&[("mcmod.info", json)]);
        let info = metadata.info().unwrap();
        assert_eq!(info.loader, Loader::LegacyForge);
        assert_eq!(info.id, "old");
        assert!(info.description.contains("line two"), "{:?}", info.description);
        assert_eq!(info.authors, vec!["X"]);
    }

    #[test]
    fn reports_missing_metadata_rather_than_guessing_from_the_filename() {
        let metadata = read_map(&[("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\n")]);
        match metadata {
            Metadata::Absent { looked_for } => {
                assert!(looked_for.contains(&"fabric.mod.json"));
            }
            other => panic!("expected Absent, got {other:?}"),
        }
    }

    #[test]
    fn refuses_metadata_without_an_id() {
        let metadata = read_map(&[("META-INF/mods.toml", "[[mods]]\ndisplayName = \"x\"\n")]);
        let reason = match metadata {
            Metadata::Invalid { reason, .. } => reason,
            other => panic!("expected Invalid, got {other:?}"),
        };
        assert!(reason.contains("modId"), "{reason}");
    }

    #[test]
    fn reads_quilt_metadata_from_its_nested_table() {
        let json = r#"{"quilt_loader": {"id": "q", "version": "1.0", "metadata": {"name": "Q"}}}"#;
        let metadata = read_map(&[("quilt.mod.json", json)]);
        let info = metadata.info().unwrap();
        assert_eq!(info.loader, Loader::Quilt);
        assert_eq!(info.id, "q");
    }

    #[test]
    fn parses_mixin_documents() {
        let doc = parse_mixin_document(
            "example.mixins.json",
            br#"{"package":"com.example.mixin","required":true,"compatibilityLevel":"JAVA_21",
                 "mixins":["BlockMixin"],"client":["HandMixin"],"refmap":"example.refmap.json"}"#,
        )
        .unwrap();
        assert_eq!(
            doc.classes(),
            vec!["com/example/mixin/BlockMixin", "com/example/mixin/HandMixin"]
        );
        assert_eq!(doc.refmap.as_deref(), Some("example.refmap.json"));
        assert_eq!(doc.compatibility_level.as_deref(), Some("JAVA_21"));
        assert!(parse_mixin_document("x.json", b"{oops").is_err());
    }

    #[test]
    fn parses_pack_mcmeta_in_both_eras() {
        let old = parse_pack_mcmeta(br#"{"pack":{"description":"x","pack_format":34}}"#).unwrap();
        assert_eq!(old.pack_format.unwrap().major, 34);
        let modern = parse_pack_mcmeta(
            br#"{"pack":{"description":{"text":"x"},"min_format":[97,1],"max_format":97}}"#,
        )
        .unwrap();
        assert_eq!(modern.min_format.unwrap().major, 97);
        assert_eq!(modern.min_format.unwrap().minor, 1);
        assert_eq!(modern.max_format.unwrap().major, 97);
        assert!(modern.description.contains("text"));
        assert!(parse_pack_mcmeta(b"{}").is_err());
    }

    #[test]
    fn forge_dependencies_split_minecraft_from_mods() {
        let toml = r#"
[[mods]]
modId = "x"
version = "1"

[[dependencies.x]]
modId = "minecraft"
versionRange = "[1.21.1,1.22)"

[[dependencies.x]]
modId = "neoforge"
versionRange = "[21.1,)"

[[dependencies.x]]
modId = "helper"
versionRange = "[2.0,)"
mandatory = false
"#;
        let metadata = read_map(&[("META-INF/neoforge.mods.toml", toml)]);
        let info = metadata.info().unwrap();
        assert_eq!(info.minecraft.len(), 1);
        assert_eq!(info.dependencies.len(), 1);
        assert_eq!(info.dependencies[0].id, "helper");
        assert!(!info.dependencies[0].required);
    }
}
