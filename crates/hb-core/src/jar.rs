//! Reading a Java mod jar into an inspectable model.
//!
//! A jar is untrusted input. Every step here is bounded: entry sizes and
//! totals, class-file counts, metadata size, and name validation. The reader
//! keeps every entry's bytes so later stages (asset census, JSON fixes,
//! packaging) do not have to reopen the file, and keeps the *names* of entries
//! it refused so the report can explain what was dropped.

use hb_classfile::{ClassFile, MixinConfig};
use hb_mcmeta::{Loader, Metadata, ModInfo, PackMeta};
use hb_zip::Archive;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::Path;

/// Limits for reading one jar.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest single entry.
    pub max_entry: u64,
    /// Largest total unpacked size.
    pub max_total: u64,
    /// Most entries.
    pub max_entries: usize,
    /// Most class files to parse.
    pub max_classes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entry: 256 << 20,
            max_total: 2 << 30,
            max_entries: 200_000,
            // Enough for the largest mods; parsing stops with a note past it.
            max_classes: 20_000,
        }
    }
}

/// A file that is not a class, metadata or mixin config: the passive
/// resources the bridge ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKind {
    BlockState,
    BlockModel,
    ItemModel,
    /// 1.21.4+ `assets/<ns>/items/<name>.json` item model definition.
    ItemDefinition,
    Texture,
    TextureMeta,
    Lang,
    /// 1.8.9-era `.lang` files.
    LangLegacy,
    SoundDefinition,
    SoundFile,
    Recipe,
    LootTable,
    Tag,
    Advancement,
    Function,
    StructureNbt,
    Worldgen,
    /// A data-driven registry entry (`enchantment/`, `damage_type/`,
    /// `jukebox_song/`, ...): no code, and the game registers it from the file.
    Registry,
    /// `data/<ns>/horizon/{block,item,recipe,loot_table,tags}/`, a Horizon
    /// content declaration.
    HorizonContent,
    DataOther,
    /// Anything else under `assets/`.
    AssetOther,
}

impl ResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::BlockState => "blockstate",
            ResourceKind::BlockModel => "block model",
            ResourceKind::ItemModel => "item model",
            ResourceKind::ItemDefinition => "item model definition",
            ResourceKind::Texture => "texture",
            ResourceKind::TextureMeta => "texture metadata",
            ResourceKind::Lang => "language file",
            ResourceKind::LangLegacy => "legacy language file",
            ResourceKind::SoundDefinition => "sounds.json",
            ResourceKind::SoundFile => "sound file",
            ResourceKind::Recipe => "recipe",
            ResourceKind::LootTable => "loot table",
            ResourceKind::Tag => "tag",
            ResourceKind::Advancement => "advancement",
            ResourceKind::Function => "function",
            ResourceKind::StructureNbt => "structure",
            ResourceKind::Worldgen => "worldgen",
            ResourceKind::Registry => "registry entry",
            ResourceKind::HorizonContent => "horizon content declaration",
            ResourceKind::DataOther => "data file",
            ResourceKind::AssetOther => "asset",
        }
    }

    /// Whether the file's bytes are text (JSON or `.lang`) rather than binary.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            ResourceKind::BlockState
                | ResourceKind::BlockModel
                | ResourceKind::ItemModel
                | ResourceKind::ItemDefinition
                | ResourceKind::Lang
                | ResourceKind::LangLegacy
                | ResourceKind::SoundDefinition
                | ResourceKind::Recipe
                | ResourceKind::LootTable
                | ResourceKind::Tag
                | ResourceKind::Advancement
                | ResourceKind::Function
                | ResourceKind::Worldgen
                | ResourceKind::Registry
                | ResourceKind::HorizonContent
                | ResourceKind::DataOther
        )
    }
}

/// One entry of a jar.
#[derive(Debug, Clone)]
pub struct JarEntry {
    pub name: String,
    pub bytes: Vec<u8>,
    pub kind: Option<ResourceKind>,
    /// Namespace (`assets/<ns>/`, `data/<ns>/`) when the path has one.
    pub namespace: Option<String>,
}

/// The full read of a jar.
#[derive(Debug, Clone)]
pub struct Jar {
    pub path: String,
    pub bytes_len: usize,
    pub sha256: String,
    pub metadata: Metadata,
    pub pack_meta: Option<PackMeta>,
    pub classes: Vec<ClassFile>,
    /// How many `.class` entries the archive holds, whether or not each one
    /// parsed and whether or not it fitted under the parse limit. This is what
    /// says "the mod is Java", which is why a jar over the limit is still
    /// reported as Java rather than as empty.
    pub class_entries: usize,
    pub mixin_configs: Vec<MixinConfig>,
    pub mixin_documents: Vec<hb_mcmeta::MixinDocument>,
    pub entries: Vec<JarEntry>,
    /// The `Main-Class` from `META-INF/MANIFEST.MF`, in binary name form.
    pub main_class: Option<String>,
    /// Problems that did not stop the read: a class that would not parse, a
    /// mixin config that did not parse, an entry over a limit.
    pub notes: Vec<String>,
    /// Entries skipped and why.
    pub skipped: BTreeMap<String, String>,
    /// Service loader files, which declare implementations by name.
    pub services: Vec<(String, Vec<String>)>,
}

impl Jar {
    pub fn info(&self) -> Option<&ModInfo> {
        self.metadata.info()
    }

    pub fn loader(&self) -> Option<&Loader> {
        self.info().map(|i| &i.loader)
    }

    /// Namespaces the jar writes content into.
    pub fn namespaces(&self) -> BTreeSet<String> {
        let mut namespaces = BTreeSet::new();
        for entry in &self.entries {
            if let Some(namespace) = &entry.namespace {
                namespaces.insert(namespace.clone());
            }
        }
        if let Some(info) = self.info() {
            namespaces.insert(info.id.clone());
        }
        namespaces
    }

    pub fn entries_of(&self, kind: ResourceKind) -> impl Iterator<Item = &JarEntry> {
        self.entries.iter().filter(move |e| e.kind == Some(kind))
    }

    /// Counts by kind, for the report.
    pub fn census(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for entry in &self.entries {
            if let Some(kind) = entry.kind {
                *counts.entry(kind.label()).or_insert(0) += 1;
            }
        }
        // `.class` is not a `ResourceKind` (it is not passive content), so it
        // is counted here from the names.
        let classes = self.class_entry_count();
        if classes > 0 {
            *counts.entry("class file").or_insert(0) += classes;
        }
        counts
    }

    /// How many `.class` entries the jar holds, parsed or not.
    pub fn class_entry_count(&self) -> usize {
        self.class_entries
    }

    pub fn find(&self, name: &str) -> Option<&JarEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Reads a jar from disk.
pub fn read_file(path: &Path) -> Result<Jar, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_bytes(&bytes, &path.display().to_string(), Limits::default())
}

/// Reads a jar from memory. `label` is used in messages.
pub fn read_bytes(bytes: &[u8], label: &str, limits: Limits) -> Result<Jar, String> {
    let mut archive = Archive::with_budget(
        Cursor::new(bytes.to_vec()),
        hb_zip::Budget {
            max_total_uncompressed: limits.max_total,
            max_entry_uncompressed: limits.max_entry,
            max_entries: limits.max_entries,
        },
    )
    .map_err(|e| format!("{label}: {e}"))?;
    let entries = archive
        .read_all()
        .map_err(|e| format!("{label}: {e}"))?;

    let mut jar = Jar {
        path: label.to_owned(),
        bytes_len: bytes.len(),
        sha256: sha256_hex(bytes),
        metadata: Metadata::Absent {
            looked_for: hb_mcmeta::METADATA_FILES.to_vec(),
        },
        pack_meta: None,
        classes: Vec::new(),
        class_entries: 0,
        mixin_configs: Vec::new(),
        mixin_documents: Vec::new(),
        entries: Vec::new(),
        main_class: None,
        notes: Vec::new(),
        skipped: BTreeMap::new(),
        services: Vec::new(),
    };

    let by_name: BTreeMap<String, &Vec<u8>> =
        entries.iter().map(|(n, b)| (n.clone(), b)).collect();

    // Metadata first: identity decides nothing about the rest of the read, but
    // the report order reads better with it first, and a jar with no metadata
    // is still fully readable (a resource pack dressed as a jar).
    jar.metadata = hb_mcmeta::read(|name| by_name.get(name).map(|b| b.to_vec()));
    if let Some(pack_mcmeta) = by_name.get("pack.mcmeta") {
        match hb_mcmeta::parse_pack_mcmeta(pack_mcmeta) {
            Ok(meta) => jar.pack_meta = Some(meta),
            Err(reason) => jar
                .notes
                .push(format!("pack.mcmeta is unreadable: {reason}")),
        }
    }
    if let Some(manifest) = by_name.get("META-INF/MANIFEST.MF") {
        let text = String::from_utf8_lossy(manifest);
        jar.main_class = hb_classfile::main_class(&text);
    }

    let mut classes_seen = 0usize;
    for (name, bytes) in &entries {
        if name.ends_with('/') {
            continue;
        }
        if name.ends_with(".class") {
            jar.class_entries += 1;
            classes_seen += 1;
            if classes_seen > limits.max_classes {
                jar.notes.push(format!(
                    "stopped reading classes after {}: the jar has more",
                    limits.max_classes
                ));
                continue;
            }
            match hb_classfile::parse(bytes) {
                Ok(class) => jar.classes.push(class),
                Err(e) => {
                    jar.skipped
                        .insert(name.clone(), format!("not a readable class file: {e}"));
                }
            }
            continue;
        }
        if name.ends_with(".mixins.json") || name.ends_with(".mixins.json5") {
            app(&mut jar.mixin_documents, name, bytes);
            if let Ok(document) = hb_mcmeta::parse_mixin_document(name, bytes) {
                jar.mixin_configs.push(MixinConfig {
                    file: name.clone(),
                    package: document.package.clone(),
                    mixins: document.mixins.clone(),
                    client: document.client.clone(),
                    server: document.server.clone(),
                    refmap: document.refmap.clone(),
                });
            } else {
                jar.skipped
                    .insert(name.clone(), "mixin configuration does not parse".into());
            }
            continue;
        }
        if let Some(service) = name.strip_prefix("META-INF/services/") {
            let text = String::from_utf8_lossy(bytes);
            jar.services.push((
                service.to_owned(),
                text.lines()
                    .map(|l| l.split('#').next().unwrap_or("").trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect(),
            ));
            continue;
        }
        let (kind, namespace) = classify(name);
        if let Some(kind) = kind {
            jar.entries.push(JarEntry {
                name: name.clone(),
                bytes: bytes.clone(),
                kind: Some(kind),
                namespace,
            });
        } else {
            // Metadata, manifests, refmaps and loader internals are expected
            // and not reported as skipped; anything else is worth a note.
            if !is_internal(name) {
                jar.skipped
                    .insert(name.clone(), "not a resource, class or known file".into());
            }
        }
    }
    Ok(jar)
}

fn app(documents: &mut Vec<hb_mcmeta::MixinDocument>, name: &str, bytes: &[u8]) {
    if let Ok(document) = hb_mcmeta::parse_mixin_document(name, bytes) {
        documents.push(document);
    }
}

/// Files that belong to the loader or build tooling rather than content.
fn is_internal(name: &str) -> bool {
    name.starts_with("META-INF/")
        || name == "pack.mcmeta"
        || name == "pack.png"
        || name.starts_with("LICENSE")
        || name.starts_with("LICENCE")
        || name.starts_with("icon.png")
        || name.ends_with(".accesswidener")
        || name.ends_with("_at.cfg")
        || name.ends_with(".refmap.json")
        || name.ends_with(".json5")
        || name.starts_with("mappings/")
}

/// Classifies a jar path and extracts its namespace.
///
/// The `recipes`/`loot_tables`/`tags/<plural>` names are recognised because
/// 1.8.9–1.20.6 jars use them, not because this tool writes them.
pub fn classify(name: &str) -> (Option<ResourceKind>, Option<String>) {
    // assets/<namespace>/...
    if let Some(rest) = name.strip_prefix("assets/") {
        let (namespace, tail) = match rest.split_once('/') {
            Some((namespace, tail)) => (namespace.to_owned(), tail),
            None => return (None, None),
        };
        let kind = if tail.starts_with("blockstates/") {
            ResourceKind::BlockState
        } else if tail.starts_with("models/block/") {
            ResourceKind::BlockModel
        } else if tail.starts_with("models/item/") {
            ResourceKind::ItemModel
        } else if tail.starts_with("items/") {
            ResourceKind::ItemDefinition
        } else if tail.starts_with("textures/") {
            if tail.ends_with(".png.mcmeta") || tail.ends_with(".mcmeta") {
                ResourceKind::TextureMeta
            } else {
                ResourceKind::Texture
            }
        } else if tail.starts_with("lang/") {
            if tail.ends_with(".lang") {
                ResourceKind::LangLegacy
            } else {
                ResourceKind::Lang
            }
        } else if tail == "sounds.json" {
            ResourceKind::SoundDefinition
        } else if tail.starts_with("sounds/") {
            ResourceKind::SoundFile
        } else {
            ResourceKind::AssetOther
        };
        return (Some(kind), Some(namespace));
    }
    // data/<namespace>/...
    if let Some(rest) = name.strip_prefix("data/") {
        let (namespace, tail) = match rest.split_once('/') {
            Some((namespace, tail)) => (namespace.to_owned(), tail),
            None => return (None, None),
        };
        let folder = tail.split('/').next().unwrap_or("");
        let kind = match folder {
            "recipe" | "recipes" => ResourceKind::Recipe,
            "loot_table" | "loot_tables" => ResourceKind::LootTable,
            "tags" => ResourceKind::Tag,
            "advancement" | "advancements" => ResourceKind::Advancement,
            "function" | "functions" => ResourceKind::Function,
            "structure" | "structures" => ResourceKind::StructureNbt,
            "worldgen" | "dimension" | "dimension_type" | "biome" | "configured_feature"
            | "placed_feature" | "template_pool" | "processor_list" | "noise_settings"
            | "density_function" | "carver" => ResourceKind::Worldgen,
            "damage_type" | "enchantment" | "trim_material" | "trim_pattern"
            | "painting_variant" | "jukebox_song" | "banner_pattern" | "instrument"
            | "wolf_variant" | "cat_variant" | "frog_variant" | "chicken_variant"
            | "cow_variant" | "pig_variant" | "chat_type" | "recipe_serializer"
            | "decorated_pot_pattern" | "armor_trim" => ResourceKind::Registry,
            // A declaration in this tool's own content format: already what
            // the target reads, so it is carried, not converted.
            "horizon" => ResourceKind::HorizonContent,
            _ => ResourceKind::DataOther,
        };
        return (Some(kind), Some(namespace));
    }
    (None, None)
}

/// Lowercase hex SHA-256 of `bytes`.
///
/// Implemented here rather than pulled in as a dependency; it is used for
/// provenance in reports (`docs/SECURITY.md` explains why SHA-512 is used for
/// `[[jar]]` references: Horizon requires it, and this tool's own file
/// digest is only for humans comparing jars).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha256(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Lowercase hex SHA-512, the form Horizon's `[[jar]]` `sha512` key requires.
pub fn sha512_hex(bytes: &[u8]) -> String {
    let digest = sha512(bytes);
    let mut out = String::with_capacity(128);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    compress_sha256(&mut h, &message, &K);
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn compress_sha256(h: &mut [u32; 8], message: &[u8], k: &[u32; 64]) {
    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    const K: [u64; 80] = [
        0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
        0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
        0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
        0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
        0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
        0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
        0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
        0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
        0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
        0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
        0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
        0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
        0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
        0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
        0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
        0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
        0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
        0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
        0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
        0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
    ];
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
        0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u128).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 128 != 112 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks_exact(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut word = [0u8; 8];
            word.copy_from_slice(&chunk[i * 8..i * 8 + 8]);
            w[i] = u64::from_be_bytes(word);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 64];
    for (i, word) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha512_hex(b"abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // 112-byte and 128-byte boundaries exercise the padding paths.
        for len in [55usize, 56, 63, 64, 111, 112, 119, 128, 1000] {
            let data = vec![b'x'; len];
            assert_eq!(sha256_hex(&data).len(), 64);
            assert_eq!(sha512_hex(&data).len(), 128);
        }
        // A known long vector for SHA-512's multi-block path.
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(&sha512_hex(&million_a)[..16], "e718483d0ce76964");
    }

    #[test]
    fn classifies_paths() {
        let cases = [
            ("assets/ruby/blockstates/ruby_block.json", "blockstate", "ruby"),
            ("assets/ruby/models/block/ruby_block.json", "block model", "ruby"),
            ("assets/ruby/models/item/ruby.json", "item model", "ruby"),
            ("assets/ruby/items/ruby.json", "item model definition", "ruby"),
            ("assets/ruby/textures/block/ruby_block.png", "texture", "ruby"),
            ("assets/minecraft/textures/gui/x.png.mcmeta", "texture metadata", "minecraft"),
            ("assets/ruby/lang/en_us.json", "language file", "ruby"),
            ("assets/ruby/lang/en_US.lang", "legacy language file", "ruby"),
            ("assets/ruby/sounds.json", "sounds.json", "ruby"),
            ("assets/ruby/sounds/block/x.ogg", "sound file", "ruby"),
            ("data/ruby/recipe/ruby_block.json", "recipe", "ruby"),
            ("data/ruby/recipes/ruby_block.json", "recipe", "ruby"),
            ("data/ruby/loot_table/blocks/a.json", "loot table", "ruby"),
            ("data/minecraft/tags/block/mineable/pickaxe.json", "tag", "minecraft"),
            ("data/ruby/advancement/x.json", "advancement", "ruby"),
            ("data/ruby/function/x.mcfunction", "function", "ruby"),
            ("data/ruby/structure/x.nbt", "structure", "ruby"),
            ("data/ruby/worldgen/biome/x.json", "worldgen", "ruby"),
            ("data/ruby/horizon/block/ruby_block.json", "horizon content declaration", "ruby"),
        ];
        for (path, label, namespace) in cases {
            let (kind, ns) = classify(path);
            assert_eq!(kind.unwrap().label(), label, "{path}");
            assert_eq!(ns.as_deref(), Some(namespace), "{path}");
        }
        assert_eq!(classify("com/example/Mod.class").0, None);
        assert_eq!(classify("META-INF/MANIFEST.MF").0, None);
    }

    #[test]
    fn textual_kinds_are_marked_text() {
        assert!(ResourceKind::Recipe.is_text());
        assert!(ResourceKind::Lang.is_text());
        assert!(!ResourceKind::Texture.is_text());
        assert!(!ResourceKind::SoundFile.is_text());
    }
}
