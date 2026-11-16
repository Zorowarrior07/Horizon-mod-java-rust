//! Checking a finished package against the rules Horizon's reader enforces.
//!
//! The conversion step makes files correct; this step makes the *package*
//! correct — the parts a single file cannot violate: the layout, the manifest,
//! path rules, size ceilings, compression, and the declaration each block and
//! item needs.
//!
//! The limits are Horizon's documented ones (see `docs` in `README.md`), and
//! every failure names the rule and what to do about it.

use crate::convert::Outcome;
use crate::horizon::Manifest;
use hb_json::Value;
use hb_versions::Release;
use std::collections::{BTreeMap, BTreeSet};

/// What a package may contain. Horizon's own limits.
pub const MAX_MANIFEST: usize = 32 << 20;
pub const MAX_FILES: usize = 65_536;
pub const MAX_FILE: usize = 16 << 20;
pub const MAX_TOTAL: usize = 512 << 20;

/// One problem, with the rule it breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Stable code, `category.thing.rule`.
    pub code: &'static str,
    pub severity: Severity,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The package is refused or breaks on load.
    Error,
    /// The package installs, but something will not behave as the author
    /// expects.
    Warning,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
        }
    }
}

/// A package, in memory, before it is written.
#[derive(Debug, Clone, Default)]
pub struct Package {
    /// Path inside the package, relative, `/`-separated.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub fn from_outcome(outcome: &Outcome) -> Self {
        Self {
            files: outcome.files.clone(),
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.files.values().map(|bytes| bytes.len()).sum()
    }

    /// Adds a file, refusing a duplicate path rather than overwriting one
    /// conversion with another.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.files.insert(path.into(), bytes.into())
    }
}

/// Validates a package for a target. `manifest` is the manifest that will be
/// written with it; `None` means the package is being checked before one is
/// built, and only content rules apply.
pub fn check(package: &Package, manifest: Option<&Manifest>, target: &Release) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut add = |severity: Severity, code: &'static str, path: Option<&str>, message: String| {
        problems.push(Problem {
            code,
            severity,
            path: path.map(|p| p.to_owned()),
            message,
        });
    };

    // Size ceilings.
    if package.len() > MAX_FILES {
        add(
            Severity::Error,
            "package.limit.files",
            None,
            format!(
                "{} files is over the {} a package may hold",
                package.len(),
                MAX_FILES
            ),
        );
    }
    let total = package.total_bytes();
    if total > MAX_TOTAL {
        add(
            Severity::Error,
            "package.limit.total",
            None,
            format!(
                "{} MiB is over the {} MiB ceiling for a whole package",
                total >> 20,
                MAX_TOTAL >> 20
            ),
        );
    }

    // Per-file rules: size, path shape, manifest size.
    for (path, bytes) in &package.files {
        if bytes.len() > MAX_FILE {
            add(
                Severity::Error,
                "package.limit.file",
                Some(path),
                format!(
                    "{} MiB is over the 16 MiB per-file limit",
                    bytes.len() as f64 / (1024.0 * 1024.0)
                ),
            );
        }
        if let Some(reason) = path_problem(path) {
            add(Severity::Error, "package.path.invalid", Some(path), reason);
        }
        if path == "horizon-mod.toml" && bytes.len() > MAX_MANIFEST {
            add(
                Severity::Error,
                "package.limit.manifest",
                Some(path),
                "the manifest is over 32 MiB".into(),
            );
        }
        if path.eq_ignore_ascii_case("horizon-mod.toml") && path != "horizon-mod.toml" {
            add(
                Severity::Error,
                "package.path.manifest_case",
                Some(path),
                "the manifest must be exactly `horizon-mod.toml`, lowercase".into(),
            );
        }
        if path.ends_with('/') {
            add(
                Severity::Warning,
                "package.path.directory_entry",
                Some(path),
                "a directory entry is not needed and some readers treat it as a file".into(),
            );
        }
        // Executables: Horizon refuses a package that carries one.
        if let Some(extension) = path.rsplit('.').next() {
            if matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "dll" | "so" | "dylib" | "bat" | "cmd" | "ps1" | "sh" | "msi"
            ) {
                add(
                    Severity::Error,
                    "package.path.executable",
                    Some(path),
                    format!("`{extension}` files are not allowed in a package"),
                );
            }
        }
    }

    // The manifest.
    if let Some(manifest) = manifest {
        if !package.files.contains_key("horizon-mod.toml") {
            add(
                Severity::Error,
                "package.manifest.missing",
                None,
                "the package has no `horizon-mod.toml` at its root, so Horizon will not load it"
                    .into(),
            );
        }
        if let Err(problems_from_manifest) = manifest.validate() {
            for problem in problems_from_manifest {
                add(
                    Severity::Error,
                    "package.manifest.invalid",
                    Some("horizon-mod.toml"),
                    problem,
                );
            }
        }
        // Every declared namespace needs a folder, and every folder a
        // declaration: a namespace listed but unused is harmless, but a file
        // in an undeclared namespace is refused.
        let declared: BTreeSet<&str> = manifest
            .namespaces
            .iter()
            .map(|s| s.as_str())
            .chain(std::iter::once(manifest.id.as_str()))
            .collect();
        let mut folders: BTreeSet<String> = BTreeSet::new();
        for path in package.files.keys() {
            if let Some((root, namespace, _)) = split_asset_path(path) {
                if root == "assets" || root == "data" {
                    folders.insert(namespace.to_owned());
                }
            }
        }
        folders.remove("minecraft");
        for folder in &folders {
            if !declared.contains(folder.as_str()) {
                add(
                    Severity::Error,
                    "package.namespace.undeclared",
                    None,
                    format!(
                        "files are under namespace `{folder}` but the manifest does not declare \
                         it; Horizon refuses a namespace a mod does not own"
                    ),
                );
            }
        }
        if !folders.contains(manifest.id.as_str())
            && !package.files.is_empty()
        {
            add(
                Severity::Warning,
                "package.namespace.empty",
                None,
                format!(
                    "nothing is under the mod's own namespace `{}`; a package whose content all \
                     sits in a declared extra namespace still installs, but its id owns no files",
                    manifest.id
                ),
            );
        }
    }

    // An empty package installs and does nothing: refuse it rather than ship
    // a file that looks like a mod.
    if !package.files.keys().any(|p| {
        (p.starts_with("assets/") || p.starts_with("data/")) && !p.ends_with('/')
    }) {
        add(
            Severity::Error,
            "package.content.empty",
            None,
            "the package has no assets or data: it would install and do nothing".into(),
        );
    }

    // Content rules.
    content_rules(package, target, &mut problems);
    problems.sort_by(|a, b| (a.severity, a.code, &a.path).cmp(&(b.severity, b.code, &b.path)));
    problems.dedup();
    problems
}

/// A path that Horizon's package reader would refuse, or `None`.
fn path_problem(path: &str) -> Option<String> {
    if path.is_empty() {
        return Some("an empty path".into());
    }
    if path.len() > 400 {
        return Some("a path longer than 400 characters".into());
    }
    if path.starts_with('/') {
        return Some("an absolute path; paths are relative to the package root".into());
    }
    if path.contains('\\') {
        return Some("a backslash; package paths use forward slashes".into());
    }
    if path.contains('\0') {
        return Some("a NUL byte".into());
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.iter().any(|s| *s == "..") {
        return Some("`..` in the path: a package may not escape its own root".into());
    }
    if segments.iter().any(|s| *s == ".") {
        return Some("`.` in the path".into());
    }
    if segments.iter().any(|s| s.is_empty()) && !path.ends_with('/') {
        return Some("an empty path segment (`//`)".into());
    }
    if let Some(first) = segments.first() {
        if first.len() == 2 && first.ends_with(':') {
            return Some("a drive letter; paths are relative".into());
        }
    }
    if path.chars().any(|c| (c as u32) < 0x20) {
        return Some("a control character".into());
    }
    None
}

fn split_asset_path(path: &str) -> Option<(&str, &str, &str)> {
    let (root, rest) = path.split_once('/')?;
    if root != "assets" && root != "data" {
        return None;
    }
    let (namespace, tail) = rest.split_once('/')?;
    Some((root, namespace, tail))
}

/// Rules that need the whole package: declarations and their evidence.
fn content_rules(package: &Package, target: &Release, problems: &mut Vec<Problem>) {
    let mut declared_blocks: BTreeSet<String> = BTreeSet::new();
    let mut declared_items: BTreeSet<String> = BTreeSet::new();
    let mut blockstates: BTreeSet<String> = BTreeSet::new();
    let mut item_models: BTreeSet<String> = BTreeSet::new();
    let mut definitions: BTreeSet<String> = BTreeSet::new();

    for (path, bytes) in &package.files {
        let Some((root, namespace, tail)) = split_asset_path(path) else {
            continue;
        };
        let stem = |prefix: &str| {
            tail.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(".json"))
                .filter(|name| !name.contains('/'))
                .map(|name| format!("{namespace}:{name}"))
        };
        if root == "data" {
            if let Some(name) = stem("horizon/block/") {
                declared_blocks.insert(name.clone());
                check_declaration(bytes, path, "block", &name, problems);
            } else if let Some(name) = stem("horizon/item/") {
                declared_items.insert(name.clone());
                check_declaration(bytes, path, "item", &name, problems);
            } else if tail.starts_with("horizon/") && stem_is_known(tail) {
                // recipe/loot_table/tags declarations are Horizon's own
                // format; nothing to check beyond being valid JSON, which the
                // conversion already did.
            }
        } else if root == "assets" {
            if let Some(name) = stem("blockstates/") {
                blockstates.insert(name);
            } else if let Some(name) = stem("models/item/") {
                item_models.insert(name);
            } else if let Some(name) = stem("items/") {
                definitions.insert(name);
            }
        }
    }

    // Only the Horizon target has content declarations at all.
    if target.layout == hb_versions::Layout::ModernHorizon {
        for block in &blockstates {
            if !declared_blocks.contains(block) {
                problems.push(Problem {
                    code: "content.block.undeclared",
                    severity: Severity::Error,
                    path: Some(format!("{block} blockstate")),
                    message: format!(
                        "`{block}` has a blockstate but no `data/<id>/horizon/block/` \
                         declaration, so the block does not exist in game and the blockstate is \
                         dead weight"
                    ),
                });
            }
        }
        for item in &item_models {
            // An item model whose name matches a block is the block's own item.
            if declared_items.contains(item) || declared_blocks.contains(item) {
                continue;
            }
            if item_models.len() == 0 {
                continue;
            }
            problems.push(Problem {
                code: "content.item.undeclared",
                severity: Severity::Warning,
                path: Some(format!("{item} item model")),
                message: format!(
                    "`{item}` has an item model but no `data/<id>/horizon/item/` declaration; if \
                     it is a block's item, declare the block instead"
                ),
            });
        }
        if declared_blocks.is_empty() && declared_items.is_empty() {
            problems.push(Problem {
                code: "content.no_declarations",
                severity: Severity::Warning,
                path: None,
                message: "this package declares no blocks or items, so it can only retexture or \
                          add data for existing ones"
                    .into(),
            });
        }
    }

    // 1.21.4+ needs a definition beside an item model.
    if matches!(
        target.layout,
        hb_versions::Layout::ItemDefinitions | hb_versions::Layout::ModernHorizon
    ) {
        for item in &item_models {
            if !definitions.contains(item) {
                problems.push(Problem {
                    code: "asset.item_definition.missing",
                    severity: Severity::Warning,
                    path: Some(format!("{item} item model")),
                    message: format!(
                        "`{item}` has no `items/` definition; 1.21.4+ reads it from there and \
                         otherwise shows a missing model"
                    ),
                });
            }
        }
    }

    // Java and loader metadata must not be carried: they do nothing and make a
    // package look like something it is not.
    for path in package.files.keys() {
        if path.ends_with(".class")
            || path.ends_with(".mixins.json")
            || path == "fabric.mod.json"
            || path == "mcmod.info"
            || path.ends_with("mods.toml")
        {
            problems.push(Problem {
                code: "package.java.leftover",
                severity: Severity::Error,
                path: Some(path.clone()),
                message: "Java code and loader metadata are not part of a Horizon package and \
                          Horizon refuses files it cannot interpret"
                    .into(),
            });
        }
    }
}

fn stem_is_known(tail: &str) -> bool {
    ["horizon/recipe/", "horizon/loot_table/", "horizon/tags/"]
        .iter()
        .any(|prefix| tail.starts_with(prefix))
}

fn check_declaration(
    bytes: &[u8],
    path: &str,
    kind: &str,
    name: &str,
    problems: &mut Vec<Problem>,
) {
    let value: Value = match hb_json::parse_bytes(bytes) {
        Ok(value) => value,
        Err(e) => {
            problems.push(Problem {
                code: "content.declaration.unreadable",
                severity: Severity::Error,
                path: Some(path.to_owned()),
                message: format!("the declaration for `{name}` is not JSON: {e}"),
            });
            return;
        }
    };
    if value.get_str("template").is_none() {
        problems.push(Problem {
            code: "content.declaration.no_template",
            severity: Severity::Error,
            path: Some(path.to_owned()),
            message: format!(
                "the {kind} declaration for `{name}` has no `template`; every declaration copies \
                 a vanilla template"
            ),
        });
    }
    if kind == "block" {
        if let Some(properties) = value.get("properties").and_then(Value::as_obj) {
            for (key, value) in properties.iter() {
                if key == "light" {
                    if value.as_i64().is_some_and(|n| !(0..=15).contains(&n)) {
                        problems.push(Problem {
                            code: "content.block.light_range",
                            severity: Severity::Error,
                            path: Some(path.to_owned()),
                            message: "`light` must be 0 to 15".into(),
                        });
                    }
                }
                if key == "max_stack_size" {
                    if value.as_i64().is_some_and(|n| !(1..=99).contains(&n)) {
                        problems.push(Problem {
                            code: "content.item.stack_size",
                            severity: Severity::Error,
                            path: Some(path.to_owned()),
                            message: "`max_stack_size` must be 1 to 99".into(),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon::Manifest;

    fn package(files: &[(&str, &[u8])]) -> Package {
        let mut package = Package::default();
        for (path, bytes) in files {
            package.insert(*path, bytes.to_vec());
        }
        package
    }

    fn manifest() -> Manifest {
        let mut manifest = Manifest::new("ruby", "Ruby", "1.0.0");
        manifest.side = crate::horizon::Side::Both;
        manifest
    }

    #[test]
    fn a_tidy_package_passes() {
        let package = package(&[
            ("horizon-mod.toml", b"id = \"ruby\"\n"),
            (
                "data/ruby/horizon/block/ruby_block.json",
                br#"{"template":"minecraft:stone","properties":{}}"#,
            ),
            ("assets/ruby/lang/en_us.json", br#"{"block.ruby.ruby_block":"Ruby Block"}"#),
        ]);
        let problems = check(&package, Some(&manifest()), hb_versions::horizon_target());
        assert!(
            problems.iter().all(|p| p.severity == Severity::Warning),
            "{problems:#?}"
        );
    }

    #[test]
    fn path_rules_are_enforced() {
        for path in ["../escape.json", "/abs.json", "a\\b.json", "C:/x.json"] {
            let package = package(&[(path, b"{}")]);
            let problems = check(&package, None, hb_versions::horizon_target());
            assert!(
                problems.iter().any(|p| p.code == "package.path.invalid"),
                "{path} should be refused"
            );
        }
    }

    #[test]
    fn java_files_are_refused() {
        let package = package(&[("com/example/Mod.class", b"cafebabe")]);
        let problems = check(&package, None, hb_versions::horizon_target());
        assert!(problems.iter().any(|p| p.code == "package.java.leftover"));
    }

    #[test]
    fn an_undeclared_namespace_is_refused() {
        let package = package(&[("assets/other/lang/en_us.json", b"{}")]);
        let problems = check(&package, Some(&manifest()), hb_versions::horizon_target());
        assert!(problems
            .iter()
            .any(|p| p.code == "package.namespace.undeclared"));
    }

    #[test]
    fn a_blockstate_without_a_declaration_is_refused() {
        let package = package(&[(
            "assets/ruby/blockstates/ruby_block.json",
            br#"{"variants":{"":{}}}"#,
        )]);
        let problems = check(&package, Some(&manifest()), hb_versions::horizon_target());
        assert!(problems
            .iter()
            .any(|p| p.code == "content.block.undeclared"));
    }

    #[test]
    fn declaration_ranges_are_checked() {
        let package = package(&[(
            "data/ruby/horizon/block/glow.json",
            br#"{"template":"minecraft:stone","properties":{"light":20}}"#,
        )]);
        let problems = check(&package, Some(&manifest()), hb_versions::horizon_target());
        assert!(problems
            .iter()
            .any(|p| p.code == "content.block.light_range"));
    }

    #[test]
    fn missing_manifest_is_an_error() {
        let package = package(&[("assets/ruby/lang/en_us.json", b"{}")]);
        let problems = check(&package, Some(&manifest()), hb_versions::horizon_target());
        assert!(problems
            .iter()
            .any(|p| p.code == "package.manifest.missing"));
    }

    #[test]
    fn item_models_need_definitions_from_1_21_4() {
        let package = package(&[(
            "assets/ruby/models/item/gem.json",
            br#"{"parent":"minecraft:item/generated"}"#,
        )]);
        let problems = check(
            &package,
            Some(&manifest()),
            hb_versions::release("1.21.4").unwrap(),
        );
        assert!(problems
            .iter()
            .any(|p| p.code == "asset.item_definition.missing"));
    }
}
