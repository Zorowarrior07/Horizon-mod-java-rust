//! One jar, one target: the whole run.
//!
//! The order matters and is the compatibility engine's definition:
//!
//! 1. **Detect** the source — loader, Minecraft version, and how sure we are.
//! 2. **Check the source profile** against the target: which layouts the jar's
//!    files are in, and which target's files they must become.
//! 3. **Convert** what has a valid conversion, and record what does not.
//! 4. **Decide the side** and build the manifest from the source's own
//!    metadata.
//! 5. **Validate** the assembled package.
//! 6. **Write** it, and report everything, including that it is untested.
//!
//! Nothing in step 3 may quietly drop a file: a refusal is a note with a code,
//! and it appears in the report.

use crate::convert::{self, Note, Options, Outcome};
use crate::detect::{self, Detection, Verdict};
use crate::horizon::{Dependency, Manifest, Side};
use crate::package::{self, Assembled, Product};
use crate::report::Report;
use crate::validate::{self, Package, Problem};
use crate::jar::Jar;
use hb_versions::Release;

/// What to do with one jar.
#[derive(Debug, Clone)]
pub struct Request {
    /// The package id. Defaults to `<mod id>_port` so it can sit beside the
    /// Java mod rather than colliding with it.
    pub package_id: Option<String>,
    pub target: &'static Release,
    /// Overrides detection. Required when the jar does not state its version.
    pub source: Option<&'static Release>,
    pub product: Product,
    /// Copies files that did not convert into `bridge-originals/`.
    pub keep_unconverted: bool,
}

impl Request {
    pub fn new(target: &'static Release) -> Self {
        Self {
            package_id: None,
            target,
            source: None,
            product: if target.layout == hb_versions::Layout::ModernHorizon {
                Product::Horizon
            } else {
                Product::JavaPack
            },
            keep_unconverted: false,
        }
    }
}

/// Everything a run produced.
pub struct Run {
    pub target: &'static Release,
    pub jar_path: String,
    pub jar_name: String,
    pub detection: Detection,
    pub verdict: (Verdict, String),
    pub outcome: Outcome,
    pub manifest: Manifest,
    pub assembled: Assembled,
    pub problems: Vec<Problem>,
    /// Where the package was written, when it was.
    pub written: Option<std::path::PathBuf>,
}

impl Run {
    pub fn report(&self) -> Report<'_> {
        Report {
            jar_path: &self.jar_path,
            jar_name: &self.jar_name,
            detection: &self.detection,
            verdict: self.verdict.clone(),
            outcome: Some(&self.outcome),
            target: self.manifest_target(),
            manifest: Some(&self.manifest),
            problems: self.problems.clone(),
            package: self.written.as_ref().map(|path| {
                (
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    self.assembled.package.len(),
                    self.assembled.package.total_bytes(),
                )
            }),
            tested_in_game: false,
        }
    }

    fn manifest_target(&self) -> &'static Release {
        self.target
    }

}

/// Runs the pipeline. The jar is read by the caller so its `Jar` can be
/// inspected first when the version must be supplied.
pub fn run(jar: &Jar, request: &Request, jar_path: &str, jar_name: &str) -> Run {
    let mut detection = detect::detect(jar);

    // An explicit source wins over detection, and the report says the
    // detection was overridden rather than pretending they agreed.
    let source = match request.source {
        Some(explicit) => {
            if detection.version.map(|v| v.id) != Some(explicit.id) {
                detection.evidence.push(format!(
                    "the source version was given as {}; detection said {}",
                    explicit.id,
                    detection
                        .version
                        .map(|v| v.id)
                        .unwrap_or("nothing")
                ));
            }
            Some(explicit)
        }
        None => detection.version,
    };

    let (verdict, explanation) = detect::content_verdict(jar, &detection);
    // A jar with no metadata still gets a manifest, from its file name: the
    // report has already said the metadata is missing.
    let mod_info = jar.info().cloned().unwrap_or_else(|| hb_mcmeta::ModInfo {
        loader: hb_mcmeta::Loader::Forge,
        id: sanitize_id(
            jar_path
                .rsplit('/')
                .next()
                .unwrap_or(jar_path)
                .trim_end_matches(".jar"),
        ),
        name: String::new(),
        version: String::new(),
        description: String::new(),
        authors: Vec::new(),
        license: String::new(),
        homepage: None,
        issues: None,
        dependencies: Vec::new(),
        minecraft: Vec::new(),
        entry_points: Vec::new(),
        mixin_configs: Vec::new(),
        access_files: Vec::new(),
        extras: Default::default(),
    });

    // The package id: the mod's id plus a suffix, so a converted package can
    // live beside the Java mod it came from.
    let default_id = format!("{}_port", sanitize_id(&mod_info.id));
    let package_id = request
        .package_id
        .clone()
        .map(|id| sanitize_id(&id))
        .unwrap_or(default_id);

    let options = Options {
        mod_id: package_id.clone(),
        target: request.target,
        source,
        keep_unconverted: request.keep_unconverted,
    };
    let outcome = convert::convert(jar, &options);

    let mut manifest = manifest_for(jar, &mod_info, &outcome, request, &package_id);
    let assembled = package::assemble(&outcome, &manifest, request.target, request.product);
    // The manifest is part of what is validated, so put it in before checking.
    let mut package = Package::from_outcome(&outcome);
    for (path, bytes) in &assembled.package.files {
        package.insert(path.clone(), bytes.clone());
    }
    if request.product == Product::JavaPack {
        // A Java pack has no manifest to validate.
        manifest.side = Side::for_content(outcome.declares_content);
    }
    let problems = validate::check(
        &package,
        Some(&manifest).filter(|_| request.product == Product::Horizon),
        request.target,
    );

    Run {
        target: request.target,
        jar_path: jar_path.to_owned(),
        jar_name: jar_name.to_owned(),
        detection,
        verdict: (verdict, explanation),
        outcome,
        manifest,
        assembled,
        problems,
        written: None,
    }
}

/// Builds the manifest from the source mod's own metadata: its name, version,
/// authors and licence travel; what a Horizon manifest cannot hold (the loader,
/// the Java dependencies) is reported, not invented.
fn manifest_for(
    jar: &Jar,
    info: &hb_mcmeta::ModInfo,
    outcome: &Outcome,
    request: &Request,
    package_id: &str,
) -> Manifest {
    let mut manifest = Manifest::new(
        package_id,
        if info.name.is_empty() {
            &info.id
        } else {
            &info.name
        },
        if info.version.is_empty() {
            "1.0.0"
        } else {
            &info.version
        },
    );
    manifest.description = info.description.clone();
    manifest.authors = info.authors.clone();
    manifest.license = info.license.clone();
    manifest.homepage = info.homepage.clone();
    manifest.issues = info.issues.clone();
    // Declaring blocks or items means every joining client needs the same
    // registries, so the side is `both`; a resource-only package can be
    // `client`.
    manifest.side = Side::for_content(outcome.declares_content);
    manifest.target_game = request.target.id.to_owned();
    manifest.target_resource_format = request.target.resource_format;
    manifest.target_data_format = request.target.data_format;

    // Extra namespaces the package ships besides its own id.
    manifest.namespaces = outcome
        .namespaces
        .iter()
        .filter(|ns| ns.as_str() != package_id)
        .cloned()
        .collect();

    // Java dependencies that have no Horizon equivalent: a mod dependency is
    // recorded so the player knows what is missing rather than finding out at
    // runtime.
    for dependency in &info.dependencies {
        if !dependency.required {
            continue;
        }
        if dependency.id == "minecraft" || dependency.id == "java" || dependency.id == "fabricloader"
            || dependency.id == "forge" || dependency.id == "neoforge" || dependency.id == "fabric-api"
        {
            continue;
        }
        manifest.dependencies.push(Dependency {
            id: sanitize_id(&dependency.id),
            version: "*".to_owned(),
            optional: false,
        });
    }
    let _ = jar;
    manifest
}

/// A valid mod id: lowercase, digits and underscores, 1 to 64 characters.
fn sanitize_id(id: &str) -> String {
    let mut out: String = id
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push_str("mod");
    }
    if out.len() > 64 {
        out.truncate(64);
    }
    if !out.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        out.insert(0, 'm');
    }
    out
}

/// Writes a run's package when the checks allowed it.
pub fn write(run: &mut Run, directory: &std::path::Path) -> Result<(), String> {
    let refused = run
        .problems
        .iter()
        .any(|p| p.severity == validate::Severity::Error);
    if refused {
        return Err(format!(
            "not written: {} package check(s) failed; the report lists them",
            run.problems
                .iter()
                .filter(|p| p.severity == validate::Severity::Error)
                .count()
        ));
    }
    if run.verdict.0 == Verdict::Nothing {
        return Err("not written: the jar has no portable content".to_owned());
    }
    if run.verdict.0 == Verdict::WontRun {
        return Err(
            "not written: nothing in this jar survives without Java, and an empty package would \
             only mislead"
                .to_owned(),
        );
    }
    let path = package::write(&run.assembled, directory)?;
    run.written = Some(path);
    Ok(())
}

/// The Java-only findings, as notes, for a caller that wants them separately.
pub fn java_notes(outcome: &Outcome) -> Vec<&Note> {
    outcome
        .notes
        .iter()
        .filter(|note| note.code.starts_with("java."))
        .collect()
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

    fn png() -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&16u32.to_be_bytes());
        out.extend_from_slice(&16u32.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// A minimal valid class file: magic, version 61 (Java 17), one Utf8 for
    /// the name, `Object` as the parent, no members.
    fn class_file() -> Vec<u8> {
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
        out.extend_from_slice(&61u16.to_be_bytes());
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

    const METADATA: &[u8] = br#"{"schemaVersion":1,"id":"gems","version":"2.1.0",
        "name":"Gems","description":"Gem blocks","authors":["Someone"],
        "license":"MIT","depends":{"minecraft":"1.21.1"}}"#;

    fn content_jar() -> Vec<u8> {
        jar_bytes(&[
            ("fabric.mod.json", METADATA.to_vec()),
            (
                "assets/gems/blockstates/ruby_block.json",
                br#"{"variants":{"":{"model":"gems:block/ruby_block"}}}"#.to_vec(),
            ),
            (
                "assets/gems/models/block/ruby_block.json",
                br#"{"parent":"minecraft:block/cube_all","textures":{"all":"gems:block/ruby_block"}}"#
                    .to_vec(),
            ),
            (
                "assets/gems/models/item/ruby.json",
                br#"{"parent":"minecraft:item/generated","textures":{"layer0":"gems:item/ruby"}}"#
                    .to_vec(),
            ),
            ("assets/gems/textures/block/ruby_block.png", png()),
            ("assets/gems/textures/item/ruby.png", png()),
            (
                "assets/gems/lang/en_us.json",
                br#"{"block.gems.ruby_block":"Block of Ruby","item.gems.ruby":"Ruby"}"#.to_vec(),
            ),
            (
                "data/gems/recipe/ruby_block.json",
                br#"{"type":"minecraft:crafting_shapeless","category":"misc","group":"","ingredients":[{"item":"gems:ruby"},{"item":"gems:ruby"}],"result":{"item":"gems:ruby_block","count":1}}"#.to_vec(),
            ),
        ])
    }

    #[test]
    fn a_content_mod_ports_to_26_3_and_writes_a_package() {
        let bytes = content_jar();
        let jar = read_bytes(&bytes, "gems.jar", Default::default()).unwrap();
        let request = Request::new(hb_versions::horizon_target());
        let mut run = run(&jar, &request, "gems.jar", "gems.jar");

        assert_eq!(run.detection.version.map(|v| v.id), Some("1.21.1"));
        assert_eq!(run.verdict.0, Verdict::Works);
        assert_eq!(run.manifest.id, "gems_port");
        assert_eq!(run.manifest.name, "Gems");
        assert_eq!(run.manifest.version, "2.1.0");
        assert_eq!(run.manifest.side, Side::Both, "declares a block");
        assert_eq!(run.manifest.target_game, "26.3");

        // The report is clean enough to write, and writing works.
        assert!(
            !run.problems
                .iter()
                .any(|p| p.severity == validate::Severity::Error),
            "{:#?}",
            run.problems
        );
        let dir = std::env::temp_dir().join("hb-bridge-test-26-3");
        let _ = std::fs::remove_dir_all(&dir);
        write(&mut run, &dir).expect("written");
        let path = run.written.clone().unwrap();
        assert!(path.exists());
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".hmod"));

        // And the file on disk is a zip carrying a manifest and declarations.
        let bytes = std::fs::read(&path).unwrap();
        let archive = hb_zip::Archive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<&str> = archive.names().collect();
        assert!(names.contains(&"horizon-mod.toml"));
        assert!(names.contains(&"bridge.json"));
        assert!(names.contains(&"data/gems_port/horizon/block/ruby_block.json"));
        assert!(names.contains(&"data/gems_port/horizon/item/ruby.json"));
        assert!(names.contains(&"assets/gems_port/textures/block/ruby_block.png"));
        assert!(!names.iter().any(|n| n.ends_with(".class")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_content_mod_makes_a_pack_for_1_21_1() {
        let bytes = content_jar();
        let jar = read_bytes(&bytes, "gems.jar", Default::default()).unwrap();
        let mut request = Request::new(hb_versions::release("1.21.1").unwrap());
        request.product = Product::JavaPack;
        let mut run = run(&jar, &request, "gems.jar", "gems.jar");
        // 1.21.1 reads the older recipe shape, so the recipe is copied, not
        // converted away.
        assert!(run
            .assembled
            .package
            .files
            .contains_key("data/gems_port/recipe/ruby_block.json"));
        assert!(run.assembled.package.files.contains_key("pack.mcmeta"));
        let dir = std::env::temp_dir().join("hb-bridge-test-1-21-1");
        let _ = std::fs::remove_dir_all(&dir);
        write(&mut run, &dir).expect("written");
        assert!(run
            .written
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("java-pack"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_jar_with_only_code_is_refused_with_its_reason() {
        let bytes = jar_bytes(&[
            ("fabric.mod.json", METADATA.to_vec()),
            // A genuine, if minimal, class file.
            (
                "com/example/Mod.class",
                class_file(),
            ),
        ]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let request = Request::new(hb_versions::horizon_target());
        let mut run = run(&jar, &request, "x.jar", "x.jar");
        assert_eq!(run.verdict.0, Verdict::WontRun);
        let error = write(&mut run, std::env::temp_dir().as_path()).unwrap_err();
        assert!(error.contains("no portable content") || error.contains("failed"), "{error}");
        assert!(run.written.is_none());
    }

    #[test]
    fn an_explicit_source_is_recorded_as_an_override() {
        let bytes = jar_bytes(&[("fabric.mod.json", METADATA.to_vec())]);
        let jar = read_bytes(&bytes, "x.jar", Default::default()).unwrap();
        let mut request = Request::new(hb_versions::horizon_target());
        request.source = hb_versions::release("1.20.1");
        let run = run(&jar, &request, "x.jar", "x.jar");
        assert!(run
            .detection
            .evidence
            .iter()
            .any(|e| e.contains("the source version was given as 1.20.1")));
    }

    #[test]
    fn ids_are_made_valid_rather_than_refused() {
        assert_eq!(sanitize_id("Gems Mod"), "gems_mod");
        assert_eq!(sanitize_id("9lives"), "m9lives");
        assert_eq!(sanitize_id(""), "mod");
        assert!(crate::horizon::valid_id(&sanitize_id("Some-Mod.2")));
    }
}
