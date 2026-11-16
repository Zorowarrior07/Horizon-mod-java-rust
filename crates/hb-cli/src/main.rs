//! `zorowarrior-horizon-bridge`: inspect a Minecraft Java mod jar, port its
//! passive content, and say honestly what did and did not survive.
//!
//! Commands:
//!
//! * `inspect` — what the jar is: loader, Minecraft version, content, and what
//!   of it Horizon cannot run. Changes nothing.
//! * `convert` — convert for one target and write a package.
//! * `build` — convert for several targets at once, one package per target.
//! * `targets` — the version database.
//! * `verify` — check a package against Horizon's rules, without building it.
//!
//! Exit codes: 0 success, 1 completed with warnings, 2 refused or failed.

use hb_core::bridge::{self, Request};
use hb_core::detect::{self, Verdict};
use hb_core::package::Product;
use hb_core::report;
use hb_core::validate::Severity;
use hb_core::{jar, VERSION};
use hb_versions::{Layout, Release};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
ZOROWARRIOR HORIZON BRIDGE - port Minecraft Java mods to Horizon and to Java Edition

USAGE
  zorowarrior-horizon-bridge <command> [options]

COMMANDS
  inspect <jar>                  Say what the jar is and what of it can be ported.
                                 Writes nothing.
  convert <jar> --target <ver>   Convert for one target and write a package.
  build <jar> --targets <list>   Convert for several targets, one package each.
  targets                        List the version database.
  verify <package>               Check a built package against Horizon's rules.

CONVERSION OPTIONS
  --target <ver>          Target release: 1.8.9, 1.21.1, 1.21.4, 26.3, or any id
                          from `targets`. Repeatable on `convert`.
  --targets <list>        Comma- or space-separated list, or `all`.
  --source-version <ver>  The Minecraft version the mod was built for, when the
                          jar does not say. Without it a version is guessed from
                          the class-file version or the file layout, and the
                          report says so.
  --id <package-id>       Package id. Defaults to `<mod id>_port`.
  --out <dir>             Output directory. Defaults to `output/`.
  --product <kind>`       `horizon` (.hmod) or `java` (resource/data pack).
                          Defaults from the target, not from a guess: only a
                          26.x target can be a Horizon package.
  --keep-unconverted      Copy files that did not convert into
                          `bridge-originals/` instead of leaving them out.
  --report-json <file>    Write the machine-readable report as well as the text.
  --quiet                 Only the summary table and errors.

EXIT CODES
  0  package(s) written, nothing wrong
  1  written with warnings that need a human
  2  refused: nothing was written, or a check failed

NOTES
  Java code, mixins and loader behaviour are never executed. They are reported
  by name, with what would have to be reimplemented. No package is tested in a
  running game: every report says UNTESTED and lists what to check by hand.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("\nrun `zorowarrior-horizon-bridge help` for usage");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<u8, String> {
    let Some(command) = args.first().map(|s| s.as_str()) else {
        print!("{USAGE}");
        return Ok(2);
    };
    if matches!(command, "help" | "-h" | "--help") {
        print!("{USAGE}");
        return Ok(0);
    }
    if matches!(command, "-V" | "--version" | "version") {
        println!("zorowarrior-horizon-bridge {VERSION}");
        return Ok(0);
    }
    let rest = &args[1..];
    match command {
        "inspect" => inspect(rest),
        "convert" => convert(rest),
        "build" => build(rest),
        "targets" => targets(rest),
        "verify" => verify(rest),
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Options parsed from the command line.
#[derive(Default)]
struct Args {
    jar: Option<PathBuf>,
    targets: Vec<String>,
    source: Option<String>,
    id: Option<String>,
    out: Option<PathBuf>,
    product: Option<Product>,
    keep_unconverted: bool,
    report_json: Option<PathBuf>,
    quiet: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let mut next = |what: &str| -> Result<String, String> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("`{what}` needs a value"))
        };
        match arg {
            "--target" => parsed.targets.push(next("--target")?),
            "--targets" => {
                let value = next("--targets")?;
                parsed
                    .targets
                    .extend(value.split([',', ' ']).filter(|s| !s.is_empty()).map(|s| s.to_owned()));
            }
            "--source-version" | "--source" => parsed.source = Some(next("--source-version")?),
            "--id" | "--package-id" => parsed.id = Some(next("--id")?),
            "--out" | "--output" => parsed.out = Some(PathBuf::from(next("--out")?)),
            "--product" => {
                let value = next("--product")?;
                parsed.product = Some(match value.as_str() {
                    "horizon" | "hmod" => Product::Horizon,
                    "java" | "pack" | "zip" => Product::JavaPack,
                    other => {
                        return Err(format!(
                            "`--product {other}`: use `horizon` or `java`"
                        ))
                    }
                });
            }
            "--keep-unconverted" => parsed.keep_unconverted = true,
            "--report-json" => parsed.report_json = Some(PathBuf::from(next("--report-json")?)),
            "--quiet" => parsed.quiet = true,
            "-h" | "--help" => return Err("help".to_owned()),
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}`"))
            }
            other => {
                if parsed.jar.is_some() {
                    return Err(format!(
                        "two inputs given: `{}` and `{other}`",
                        parsed.jar.as_ref().unwrap().display()
                    ));
                }
                parsed.jar = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    Ok(parsed)
}

fn read_jar(path: &Path) -> Result<([u8; 0], hb_core::jar::Jar), String> {
    let jar = jar::read_file(path)?;
    Ok(([], jar))
}

fn jar_names(path: &Path) -> (String, String) {
    (
        path.display().to_string(),
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    )
}

fn source_release(text: &str) -> Result<&'static Release, String> {
    hb_versions::parse_version(text).ok_or_else(|| {
        format!(
            "`{text}` is not a Minecraft version this build knows; run `targets` to see them"
        )
    })
}

fn inspect(args: &[String]) -> Result<u8, String> {
    let parsed = parse(args)?;
    let path = parsed
        .jar
        .ok_or("`inspect` needs a jar: inspect <jar>")?;
    let (_, jar) = read_jar(&path)?;
    let detection = detect::detect(&jar);
    let (verdict, explanation) = detect::content_verdict(&jar, &detection);
    let (full, name) = jar_names(&path);

    let mut out = String::new();
    out.push_str("ZOROWARRIOR HORIZON BRIDGE - INSPECT\n");
    out.push_str(&format!("  {name} ({full})\n"));
    out.push_str(&format!(
        "  {} bytes, sha256 {}\n\n",
        jar.bytes_len,
        &jar.sha256[..16.min(jar.sha256.len())]
    ));
    out.push_str("SOURCE\n");
    out.push_str(&format!("  loader     {}\n", detection.loader_label()));
    if let Some(info) = &detection.metadata {
        out.push_str(&format!(
            "  mod        {} {} \"{}\"\n",
            info.id, info.version, info.name
        ));
        if !info.authors.is_empty() {
            out.push_str(&format!("  authors    {}\n", info.authors.join(", ")));
        }
        if !info.license.is_empty() {
            out.push_str(&format!("  license    {}\n", info.license));
        }
        if !info.minecraft.is_empty() {
            let wanted: Vec<String> = info
                .minecraft
                .iter()
                .map(|d| d.requirement.clone())
                .collect();
            out.push_str(&format!("  requires   minecraft {}\n", wanted.join(", ")));
        }
        if !info.dependencies.is_empty() {
            let deps: Vec<String> = info
                .dependencies
                .iter()
                .map(|d| {
                    format!(
                        "{} {}{}",
                        d.id,
                        d.requirement,
                        if d.required { "" } else { " (optional)" }
                    )
                })
                .collect();
            out.push_str(&format!("  depends    {}\n", deps.join(", ")));
        }
        if !info.entry_points.is_empty() {
            let entries: Vec<String> = info
                .entry_points
                .iter()
                .map(|e| format!("{}: {}", e.kind, e.class))
                .collect();
            out.push_str(&format!("  entry      {}\n", entries.join(", ")));
        }
    }
    out.push_str(&format!("  detection  {}\n", detection.headline()));
    for evidence in &detection.evidence {
        out.push_str(&format!("             - {evidence}\n"));
    }
    out.push('\n');

    out.push_str("CONTENT\n");
    for (kind, count) in detect::content_census(&jar) {
        // Class files get their own line below, with how many parsed.
        if kind == "class file" {
            continue;
        }
        out.push_str(&format!("  {count:>5}  {kind}\n"));
    }
    if jar.class_entries > 0 {
        out.push_str(&format!(
            "  {:>5}  class file ({} parsed, {} mixin class)\n",
            jar.class_entries,
            jar.classes.len(),
            jar.mixin_configs.len()
        ));
    }
    out.push('\n');

    out.push_str(&format!(
        "VERDICT FOR HORIZON (Minecraft {}):  {} - {}\n",
        hb_versions::horizon_target().id,
        verdict.label(),
        explanation
    ));
    if verdict != Verdict::Works {
        out.push_str("  What cannot run:\n");
        if !jar.classes.is_empty() {
            let census = hb_classfile::census(jar.classes.iter());
            for line in &census.behaviour {
                out.push_str(&format!("    - {line}\n"));
            }
            for line in &census.entry_points {
                out.push_str(&format!("    - entry point {line}\n"));
            }
            for library in &census.libraries {
                out.push_str(&format!("    - Java library {library}\n"));
            }
        }
        for (config, classes) in hb_classfile::summarize_mixins(&jar.mixin_configs) {
            out.push_str(&format!(
                "    - mixins in {config} ({} class(es)): {} rewrite game classes at load time\n",
                classes.len(),
                classes.join(", ")
            ));
        }
        out.push_str(
            "  None of this is executed or reproduced: Horizon does not run Java or mixins.\n",
        );
    }
    out.push('\n');

    if !detection.concerns.is_empty() {
        out.push_str("BEFORE YOU RELY ON THIS\n");
        for (code, message) in &detection.concerns {
            out.push_str(&format!("  * {message}\n      code {code}\n"));
        }
        out.push('\n');
    }

    out.push_str("TARGETS THIS JAR COULD BE CONVERTED FOR\n");
    let source = detection.version;
    for target in hb_versions::targets() {
        let product = if target.layout == Layout::ModernHorizon {
            Product::Horizon
        } else {
            Product::JavaPack
        };
        let note = match source {
            Some(version) if version.id == target.id => "same version",
            Some(version) if target.same_format_as(version) => {
                "same formats, no conversion needed"
            }
            _ => "conversion required",
        };
        out.push_str(&format!(
            "  {:<10} {:<10} {:<28} {note}\n",
            target.id,
            target.layout.label(),
            product.label()
        ));
    }
    print!("{out}");

    Ok(match verdict {
        Verdict::Works | Verdict::Partly => 0,
        Verdict::WontRun | Verdict::Nothing => 2,
    })
}

fn targets(args: &[String]) -> Result<u8, String> {
    if !args.is_empty() && args[0] != "--json" {
        return Err("`targets` takes only `--json`".to_owned());
    }
    if args.first().map(|s| s.as_str()) == Some("--json") {
        println!("{}", hb_json::to_string_pretty(&hb_versions::database_json()));
        return Ok(0);
    }
    println!("{:<10} {:<8} {:<10} {:<8} {:<16} {:<6} {}", "VERSION", "FAMILY", "RESOURCE", "DATA", "LAYOUT", "RELEASED", "PRODUCT");
    for release in hb_versions::targets() {
        let product = if release.layout == Layout::ModernHorizon {
            "horizon .hmod"
        } else {
            "java pack"
        };
        println!(
            "{:<10} {:<8} {:<10} {:<8} {:<16} {:<10} {}",
            release.id,
            release.family,
            release.resource_format.to_string_short(),
            release.data_format.to_string_short(),
            release.layout.label(),
            release.released,
            product
        );
    }
    println!();
    println!("A Horizon package and a Java Edition pack are different products: a `.hmod` is read by");
    println!("Horizon {} and nothing else; a Java pack is read by the game and cannot register a", hb_versions::horizon_target().id);
    println!("mod's blocks. One binary is never assumed to work on every version.");
    Ok(0)
}

fn convert(args: &[String]) -> Result<u8, String> {
    let parsed = parse(args)?;
    let path = parsed
        .jar
        .clone()
        .ok_or("`convert` needs a jar: convert <jar> --target <version>")?;
    if parsed.targets.is_empty() {
        return Err(
            "`convert` needs a target: --target 26.3 (run `targets` for the list)".to_owned(),
        );
    }
    build_targets(&path, &parsed)
}

fn build(args: &[String]) -> Result<u8, String> {
    let mut parsed = parse(args)?;
    if parsed.targets.is_empty() {
        return Err("`build` needs targets: --targets all, or --targets 1.21.1,26.3".to_owned());
    }
    if parsed.targets.iter().any(|t| t == "all") {
        parsed.targets = hb_versions::targets().map(|r| r.id.to_owned()).collect();
    }
    let path = parsed
        .jar
        .clone()
        .ok_or("`build` needs a jar: build <jar> --targets all")?;
    build_targets(&path, &parsed)
}

fn build_targets(path: &Path, parsed: &Args) -> Result<u8, String> {
    let jar = jar::read_file(path)?;
    let (full, name) = jar_names(path);
    let source = match &parsed.source {
        Some(text) => Some(source_release(text)?),
        None => None,
    };
    let out_dir = parsed
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("output"));

    let mut releases: Vec<&'static Release> = Vec::new();
    for text in &parsed.targets {
        releases.push(source_release(text)?);
    }
    // Deduplicate while keeping the order given.
    let mut seen = std::collections::BTreeSet::new();
    releases.retain(|r| seen.insert(r.id));

    let mut summaries: Vec<Summary> = Vec::new();
    let mut worst = 0u8;
    let mut table = String::new();
    table.push_str(&report::table_header());
    table.push('\n');

    for target in releases {
        let product = parsed.product.unwrap_or(if target.layout == Layout::ModernHorizon {
            Product::Horizon
        } else {
            Product::JavaPack
        });
        if product == Product::Horizon && target.layout != Layout::ModernHorizon {
            eprintln!(
                "error: Minecraft {} cannot take a Horizon `.hmod`: its content format is 26.x's. \
                 Use --product java, or --target {}",
                target.id,
                hb_versions::horizon_target().id
            );
            worst = worst.max(2);
            continue;
        }
        let request = Request {
            package_id: parsed.id.clone(),
            target,
            source,
            product,
            keep_unconverted: parsed.keep_unconverted,
        };
        let mut outcome = bridge::run(&jar, &request, &full, &name);
        // A `--out` directory that is not writable is a failure, not a warning.
        match bridge::write(&mut outcome, out_dir.as_path()) {
            Ok(()) => {}
            Err(message) => {
                if !message.starts_with("not written: the jar has no portable content")
                    && !message.starts_with("not written: nothing in this jar")
                {
                    eprintln!("error: {message}");
                }
            }
        }
        {
            let report = outcome.report();
            let code = report.exit_code();
            worst = worst.max(code as u8);
            if !parsed.quiet {
                println!("{}", report.text());
            }
            table.push_str(&report.table_row());
            table.push('\n');
            if let Some(path) = &parsed.report_json {
                let json = report.json();
                let text = hb_json::to_string_pretty(&json);
                // One target per file: two targets are two different products.
                let file = sibling(path, report.target.id);
                std::fs::write(&file, text)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
            }
            summaries.push(Summary {
                jar_name: name.clone(),
                target: target.id,
                written: report.package.as_ref().map(|(n, _, _)| n.clone()),
                refused: code == 2,
            });
        }
    }

    println!("{table}");
    for summary in &summaries {
        if summary.refused {
            println!(
                "REFUSED  {} for Minecraft {}: nothing was written; the report above says why",
                summary.jar_name, summary.target
            );
        }
    }
    let written = summaries.iter().filter(|s| s.written.is_some()).count();
    if written > 0 {
        println!(
            "\n{written} package(s) written to {}. Each is for the Minecraft version in its file \
             name and is not interchangeable with the others.",
            out_dir.display()
        );
    }
    println!(
        "Every package is UNTESTED: no Minecraft client is available to this tool. The report \
         lists what to check by hand."
    );
    Ok(worst)
}

/// One line of the batch summary, kept after the report it came from is gone.
struct Summary {
    jar_name: String,
    target: &'static str,
    written: Option<String>,
    refused: bool,
}

fn sibling(path: &Path, target: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report".to_owned());
    let extension = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "json".to_owned());
    path.with_file_name(format!("{stem}-{target}.{extension}"))
}

fn verify(args: &[String]) -> Result<u8, String> {
    let parsed = parse(args)?;
    let path = parsed
        .jar
        .ok_or("`verify` needs a package: verify <package.hmod>")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let archive = hb_zip::Archive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("{} is not a zip: {e}", path.display()))?;

    let mut package = hb_core::validate::Package::default();
    let mut names: Vec<String> = archive.names().map(|n| n.to_owned()).collect();
    names.sort();
    let mut bytes_total = 0usize;
    let mut archive = archive;
    for name in &names {
        let data = archive
            .read(name)
            .map_err(|e| format!("{name}: {e}"))?;
        bytes_total += data.len();
        package.insert(name.clone(), data);
    }

    let mut problems = Vec::new();
    // The manifest travels in the package; parse it back rather than trusting
    // the copy we built it from.
    let manifest = package
        .files
        .get("horizon-mod.toml")
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    if let Some(text) = &manifest {
        let table = hb_mcmeta::toml::parse(text).map_err(|e| format!("manifest: {e}"))?;
        let mut found = hb_core::horizon::Manifest::new("x", "x", "1");
        let mut checkable = true;
        for key in ["id", "name", "version", "description"] {
            if let Some(value) = table.get(key).and_then(|v| v.as_str()) {
                match key {
                    "id" => found.id = value.to_owned(),
                    "name" => found.name = value.to_owned(),
                    "version" => found.version = value.to_owned(),
                    _ => found.description = value.to_owned(),
                }
            } else if key == "id" || key == "name" || key == "version" {
                problems.push(hb_core::validate::Problem {
                    code: "package.manifest.invalid",
                    severity: Severity::Error,
                    path: Some("horizon-mod.toml".into()),
                    message: format!("`{key}` is missing or not a string"),
                });
                checkable = false;
            }
        }
        if let Some(api) = table.get("api").and_then(|v| v.as_str()) {
            if api != hb_core::horizon::API {
                problems.push(hb_core::validate::Problem {
                    code: "package.manifest.api",
                    severity: Severity::Error,
                    path: Some("horizon-mod.toml".into()),
                    message: format!(
                        "api `{api}` is not {}; Horizon refuses a package for another API",
                        hb_core::horizon::API
                    ),
                });
            }
        }
        if checkable {
            problems.extend(hb_core::validate::check(
                &package,
                Some(&found),
                hb_versions::horizon_target(),
            ));
        }
    } else {
        problems.extend(hb_core::validate::check(
            &package,
            None,
            hb_versions::horizon_target(),
        ));
        problems.push(hb_core::validate::Problem {
            code: "package.manifest.missing",
            severity: Severity::Error,
            path: None,
            message: "no `horizon-mod.toml` at the package root".into(),
        });
    }

    println!(
        "{}: {} file(s), {} bytes, compression {}",
        path.display(),
        package.len(),
        bytes_total,
        if archive.entries().iter().any(|e| e.method == 8) {
            "deflate"
        } else {
            "stored"
        }
    );
    let errors = problems
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .count();
    for problem in &problems {
        println!(
            "  {} {}{}: {}",
            problem.severity.label(),
            problem.code,
            problem
                .path
                .as_ref()
                .map(|p| format!(" [{p}]"))
                .unwrap_or_default(),
            problem.message
        );
    }
    if errors == 0 {
        println!("  RESULT: passes the package rules Horizon documents");
        println!(
            "  (This is a check against documented rules, not a test in a running game.)"
        );
        Ok(if problems.is_empty() { 0 } else { 1 })
    } else {
        println!("  RESULT: {} error(s); Horizon would refuse this package", errors);
        Ok(2)
    }
}
