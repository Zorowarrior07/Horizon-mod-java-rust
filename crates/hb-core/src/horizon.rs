//! Writing Horizon packages: `horizon-mod.toml` and the `.hmod` archive.
//!
//! Everything here follows the official manifest reference
//! (<https://github.com/horizon-rust/horizon/blob/main/docs/modding/src/reference/manifest.md>)
//! and the content-mod guide (<https://playhorizon.dev/docs/content-mods>):
//!
//! * `id` is `[a-z0-9_]{1,64}`; it is also the resource namespace;
//! * `api = "0.2"` is checked for exact equality;
//! * `capabilities` must be `["registries"]` for a content mod;
//! * `side` defaults to **`client`**, and a mod meant for both halves must say
//!   `both` — a silent default here would make a mod that adds blocks refuse
//!   every LAN join;
//! * unknown keys are refused by the game, so this writer only emits keys the
//!   reference documents.
//!
//! The writer is deliberately a small, explicit type rather than a `serde`
//! derive: the set of keys a converted mod may carry is a safety boundary, and
//! a struct with `deny_unknown_fields` gives that in one place.

use hb_json::Value;
use hb_versions::PackFormat;
use std::collections::BTreeMap;

/// The content API this tool writes.
pub const API: &str = "0.2";

/// The idle limit for a mod id.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        // A leading digit is allowed by the reference (`[a-z0-9_]{1,64}`).
        && id != "_"
}

/// Namespaces a mod may not claim: the game owns them.
pub const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "horizon"];

/// Which installation a mod is for. The default in the manifest is `client`,
/// and getting this wrong changes whether LAN joins are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Client,
    Both,
    Server,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Both => "both",
            Side::Server => "server",
        }
    }

    /// A content mod that adds blocks or items must be `both`: every joining
    /// client needs the same registry contents, so Horzion refuses a join when
    /// the lists differ.
    pub fn for_content(declares_registry_content: bool) -> Side {
        if declares_registry_content {
            Side::Both
        } else {
            Side::Client
        }
    }
}

/// A `[[jar]]` reference to an original Java jar (Horizon's external asset
/// source). The jar's bytes stay outside the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JarRef {
    pub id: String,
    pub version: String,
    /// Lowercase hex SHA-512, 128 characters.
    pub sha512: String,
}

impl JarRef {
    pub fn is_valid(&self) -> bool {
        self.sha512.len() == 128
            && self
                .sha512
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}

/// A dependency on another Horizon mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub version: String,
    pub optional: bool,
}

/// A complete manifest, as this tool is willing to write one.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub license_url: Option<String>,
    pub credits: String,
    pub homepage: Option<String>,
    pub issues: Option<String>,
    pub side: Side,
    /// Extra namespaces the mod's files use besides its id.
    pub namespaces: Vec<String>,
    pub dependencies: Vec<Dependency>,
    pub jars: Vec<JarRef>,
    /// The Minecraft version whose formats this package's files are in. Not a
    /// manifest key: recorded in the report and in the package's own
    /// `bridge.json`, never in `horizon-mod.toml`, because the game refuses
    /// unknown keys.
    pub target_game: String,
    pub target_resource_format: PackFormat,
    pub target_data_format: PackFormat,
}

impl Manifest {
    pub fn new(id: &str, name: &str, version: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            description: String::new(),
            authors: Vec::new(),
            license: String::new(),
            license_url: None,
            credits: String::new(),
            homepage: None,
            issues: None,
            side: Side::Client,
            namespaces: Vec::new(),
            dependencies: Vec::new(),
            jars: Vec::new(),
            target_game: String::new(),
            target_resource_format: PackFormat::major_only(97),
            target_data_format: PackFormat::major_only(121),
        }
    }

    /// Checks everything the game checks, so a package is never written that
    /// would be refused on install.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if !valid_id(&self.id) {
            problems.push(format!(
                "id `{}` is not [a-z0-9_]{{1,64}}; it is the mod's namespace and settings key",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            problems.push("name is empty; the Mods screen would show a blank row".into());
        }
        if self.version.trim().is_empty() {
            problems.push("version is empty".into());
        }
        if let Some(url) = &self.license_url {
            if !is_http(url) {
                problems.push(format!("license_url `{url}` must be http or https"));
            }
        }
        for (key, url) in [("homepage", &self.homepage), ("issues", &self.issues)] {
            if let Some(url) = url {
                if !is_http(url) || url.len() > 2048 {
                    problems.push(format!(
                        "{key} `{url}` must be an http(s) URL of at most 2048 characters"
                    ));
                }
            }
        }
        for namespace in &self.namespaces {
            if RESERVED_NAMESPACES.contains(&namespace.as_str()) {
                problems.push(format!(
                    "namespace `{namespace}` is reserved (minecraft, c and horizon cannot be claimed)"
                ));
            }
            if namespace == &self.id {
                problems.push(format!(
                    "namespace `{namespace}` repeats the mod id; list only *extra* namespaces"
                ));
            }
            if !namespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-. ".contains(&b))
                && !namespace.contains('/')
            {
                problems.push(format!("namespace `{namespace}` is not a valid resource namespace"));
            }
        }
        for dependency in &self.dependencies {
            if !valid_id(&dependency.id) {
                problems.push(format!(
                    "dependency id `{}` is not a valid mod id",
                    dependency.id
                ));
            }
            if dependency.id == self.id {
                problems.push("a mod cannot depend on itself".into());
            }
        }
        for jar in &self.jars {
            if !jar.is_valid() {
                problems.push(format!(
                    "[[jar]] `{}` needs a 128-character lowercase hex sha512",
                    jar.id
                ));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Renders `horizon-mod.toml`.
    ///
    /// Field order follows the reference documentation so a human diff reads
    /// like the docs. Optional keys are omitted when empty, never written as
    /// empty strings, because the game refuses a malformed link but shows an
    /// absent one as "not given".
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("id = {}\n", toml_string(&self.id)));
        out.push_str(&format!("name = {}\n", toml_string(&self.name)));
        out.push_str(&format!("version = {}\n", toml_string(&self.version)));
        if !self.description.is_empty() {
            out.push_str(&format!(
                "description = {}\n",
                toml_string(&self.description)
            ));
        }
        if !self.authors.is_empty() {
            out.push_str(&format!("authors = {}\n", toml_array(&self.authors)));
        }
        if !self.license.is_empty() {
            out.push_str(&format!("license = {}\n", toml_string(&self.license)));
        }
        if let Some(url) = &self.license_url {
            out.push_str(&format!("license_url = {}\n", toml_string(url)));
        }
        if !self.credits.is_empty() {
            out.push_str(&format!("credits = {}\n", toml_string(&self.credits)));
        }
        if let Some(url) = &self.homepage {
            out.push_str(&format!("homepage = {}\n", toml_string(url)));
        }
        if let Some(url) = &self.issues {
            out.push_str(&format!("issues = {}\n", toml_string(url)));
        }
        out.push_str(&format!("api = {}\n", toml_string(API)));
        out.push_str(&format!("side = {}\n", toml_string(self.side.as_str())));
        out.push_str("capabilities = [\"registries\"]\n");
        if !self.namespaces.is_empty() {
            out.push_str(&format!("namespaces = {}\n", toml_array(&self.namespaces)));
        }
        if !self.dependencies.is_empty() {
            out.push('\n');
            out.push_str("[dependencies]\n");
            for dependency in &self.dependencies {
                if dependency.optional {
                    out.push_str(&format!(
                        "{} = {{ version = {}, optional = true }}\n",
                        toml_string(&dependency.id),
                        toml_string(&dependency.version)
                    ));
                } else {
                    out.push_str(&format!(
                        "{} = {}\n",
                        toml_string(&dependency.id),
                        toml_string(&dependency.version)
                    ));
                }
            }
        }
        if !self.jars.is_empty() {
            for jar in &self.jars {
                out.push('\n');
                out.push_str("[[jar]]\n");
                out.push_str(&format!("id = {}\n", toml_string(&jar.id)));
                out.push_str(&format!("version = {}\n", toml_string(&jar.version)));
                out.push_str(&format!("sha512 = {}\n", toml_string(&jar.sha512)));
            }
        }
        out
    }

    /// The `bridge.json` sidecar: what the bridge knows that the manifest
    /// cannot say. Horizon ignores extra files at the package root that it
    /// does not recognise *only* if they are not executable, so this is a
    /// plain JSON file and is never named `mod.wasm`.
    pub fn sidecar_json(&self, provenance: &BTreeMap<String, String>) -> Value {
        let mut source = BTreeMap::new();
        for (key, value) in provenance {
            source.insert(key.clone(), Value::str(value.clone()));
        }
        Value::obj([
            (
                "schema",
                Value::str("zoro-horizon-bridge/package@1"),
            ),
            ("mod_id", Value::str(self.id.clone())),
            ("mod_version", Value::str(self.version.clone())),
            ("api", Value::str(API)),
            ("side", Value::str(self.side.as_str())),
            ("target_game", Value::str(self.target_game.clone())),
            (
                "target_resource_format",
                Value::str(self.target_resource_format.to_string_short()),
            ),
            (
                "target_data_format",
                Value::str(self.target_data_format.to_string_short()),
            ),
            (
                "provenance",
                Value::Obj(source.into_iter().collect()),
            ),
        ])
    }
}

fn is_http(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// A TOML basic string. Control characters are escaped so a description with
/// a newline cannot break the manifest.
pub fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn toml_array(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| toml_string(i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut manifest = Manifest::new("ruby", "Ruby", "0.1.0");
        manifest.description = "A ruby gem and a block of ruby".into();
        manifest.authors = vec!["You".into()];
        manifest.side = Side::Both;
        manifest
    }

    #[test]
    fn writes_the_documented_manifest() {
        let toml = sample().to_toml();
        let expected = "\
id = \"ruby\"
name = \"Ruby\"
version = \"0.1.0\"
description = \"A ruby gem and a block of ruby\"
authors = [\"You\"]
api = \"0.2\"
side = \"both\"
capabilities = [\"registries\"]
";
        assert_eq!(toml, expected);
    }

    #[test]
    fn omits_empty_optional_keys() {
        let toml = Manifest::new("x", "X", "1").to_toml();
        assert!(!toml.contains("license"));
        assert!(!toml.contains("namespaces"));
        assert!(!toml.contains("[dependencies]"));
        assert!(!toml.contains("[[jar]]"));
        assert!(toml.contains("side = \"client\""), "side defaults to client");
    }

    #[test]
    fn writes_dependencies_jars_and_namespaces() {
        let mut manifest = sample();
        manifest.namespaces = vec!["kaisyn".into()];
        manifest.dependencies = vec![
            Dependency {
                id: "base".into(),
                version: "^1.2".into(),
                optional: false,
            },
            Dependency {
                id: "extra".into(),
                version: "^2".into(),
                optional: true,
            },
        ];
        manifest.jars = vec![JarRef {
            id: "ruby_java".into(),
            version: "2.0".into(),
            sha512: "a".repeat(128),
        }];
        let toml = manifest.to_toml();
        assert!(toml.contains("namespaces = [\"kaisyn\"]"));
        assert!(toml.contains("\"base\" = \"^1.2\""));
        assert!(toml.contains("\"extra\" = { version = \"^2\", optional = true }"));
        assert!(toml.contains("[[jar]]"));
        assert!(toml.contains(&format!("sha512 = \"{}\"", "a".repeat(128))));
        manifest.validate().unwrap();
    }

    #[test]
    fn refuses_what_the_game_refuses() {
        let mut manifest = sample();
        manifest.id = "Bad-Id".into();
        assert!(manifest.validate().unwrap_err()[0].contains("a-z0-9_"));
        manifest.id = "ok".into();
        manifest.namespaces = vec!["minecraft".into()];
        assert!(manifest.validate().unwrap_err()[0].contains("reserved"));
        manifest.namespaces = vec!["ok".into()];
        assert!(manifest.validate().unwrap_err()[0].contains("repeats the mod id"));
        manifest.namespaces.clear();
        manifest.homepage = Some("ftp://example.org".into());
        assert!(manifest.validate().unwrap_err()[0].contains("http"));
        manifest.homepage = None;
        manifest.jars = vec![JarRef {
            id: "x".into(),
            version: "1".into(),
            sha512: "A".repeat(128),
        }];
        assert!(manifest.validate().unwrap_err()[0].contains("lowercase hex"));
        manifest.jars.clear();
        manifest.validate().unwrap();
    }

    #[test]
    fn escapes_descriptions() {
        let mut manifest = sample();
        manifest.description = "line \"one\"\nline two \\ end".into();
        let toml = manifest.to_toml();
        assert!(toml.contains(r#"description = "line \"one\"\nline two \\ end""#));
        // The written manifest parses back through our own reader.
        let parsed = hb_mcmeta::toml::parse(&toml).unwrap();
        assert_eq!(parsed.get_str("description"), Some(manifest.description.as_str()));
    }

    #[test]
    fn side_rules_for_content() {
        assert_eq!(Side::for_content(true), Side::Both);
        assert_eq!(Side::for_content(false), Side::Client);
        assert_eq!(Side::Both.as_str(), "both");
    }

    #[test]
    fn validates_ids() {
        assert!(valid_id("ruby"));
        assert!(valid_id("ruby_mod_2"));
        assert!(!valid_id("Ruby"));
        assert!(!valid_id("has-dash"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(65)));
        assert!(valid_id(&"a".repeat(64)));
        assert!(!valid_id("with space"));
    }

    #[test]
    fn sidecar_carries_what_the_manifest_cannot() {
        let mut manifest = sample();
        manifest.target_game = "26.3".into();
        let mut provenance = BTreeMap::new();
        provenance.insert("source_jar".into(), "ruby-2.0.jar".into());
        provenance.insert("source_sha256".into(), "abc".into());
        let json = manifest.sidecar_json(&provenance);
        assert_eq!(json.get_str("target_game"), Some("26.3"));
        assert_eq!(
            json.get("provenance").unwrap().get_str("source_jar"),
            Some("ruby-2.0.jar")
        );
        // And it is valid JSON.
        let text = hb_json::to_string_pretty(&json);
        hb_json::parse(&text).unwrap();
    }
}
