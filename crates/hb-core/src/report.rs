//! The report: what a conversion found, what it changed, what it refused, and
//! what it could not check.
//!
//! The report is the deliverable as much as the package is. It has to be
//! readable by a person who wants to know whether their mod works, and
//! machine-readable by a script that wants to fail a build — so both forms are
//! produced from one structure, never written twice by hand.
//!
//! Three things are always present and never softened:
//!
//! * the **verdict** for the target (`WORKS` / `PARTLY` / `DOESN'T RUN`),
//! * every **unsupported feature** by name, with why,
//! * whether the output was **tested against a running game**. It never has
//!   been: no Minecraft client is available to this tool, so every package says
//!   `UNTESTED` and what to check by hand.

use crate::convert::{Note, Outcome, Severity};
use crate::detect::{Detection, Verdict};
use crate::horizon::Manifest;
use crate::validate::Problem;
use hb_versions::Release;

/// One conversion's whole result, ready to render.
pub struct Report<'a> {
    pub jar_path: &'a str,
    pub jar_name: &'a str,
    pub detection: &'a Detection,
    pub verdict: (Verdict, String),
    pub outcome: Option<&'a Outcome>,
    pub target: &'a Release,
    pub manifest: Option<&'a Manifest>,
    pub problems: Vec<Problem>,
    pub package: Option<(String, usize, usize)>,
    /// Whether a running game confirmed the output. Always `false` here, and
    /// the report says so rather than leaving it out.
    pub tested_in_game: bool,
}

impl Report<'_> {
    /// The report's exit code: 0 when a package was written and nothing is
    /// wrong, 1 when it was written with warnings, 2 when it was refused.
    pub fn exit_code(&self) -> i32 {
        if self.problems.iter().any(|p| p.severity == crate::validate::Severity::Error)
            || self.verdict.0 == Verdict::WontRun
            || self.verdict.0 == Verdict::Nothing
        {
            2
        } else if self.package.is_some() {
            0
        } else {
            1
        }
    }

    /// The plain-text report: for a terminal, and for the reader who wants the
    /// answer first.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let line = |out: &mut String, s: &str| {
            out.push_str(s);
            out.push('\n');
        };

        line(&mut out, "ZOROWARRIOR HORIZON BRIDGE");
        line(
            &mut out,
            &format!("  {} ({})", self.jar_name, self.jar_path),
        );
        line(&mut out, "");
        line(&mut out, "SOURCE");
        line(&mut out, &format!("  loader     {}", self.detection.loader_label()));
        if let Some(info) = &self.detection.metadata {
            line(
                &mut out,
                &format!(
                    "  mod        {} {} \"{}\"",
                    info.id, info.version, info.name
                ),
            );
            if !info.authors.is_empty() {
                line(&mut out, &format!("  authors    {}", info.authors.join(", ")));
            }
            if !info.license.is_empty() {
                line(&mut out, &format!("  license    {}", info.license));
            }
        }
        line(&mut out, &format!("  detection  {}", self.detection.headline()));
        for evidence in &self.detection.evidence {
            line(&mut out, &format!("             - {evidence}"));
        }
        line(&mut out, "");

        line(&mut out, "TARGET");
        line(
            &mut out,
            &format!(
                "  Minecraft {} (family {}, {})",
                self.target.id,
                self.target.family,
                self.target.layout.label()
            ),
        );
        line(
            &mut out,
            &format!(
                "  formats    resource {} / data {}",
                self.target.resource_format.to_string_short(),
                self.target.data_format.to_string_short()
            ),
        );
        line(&mut out, "");

        line(
            &mut out,
            &format!(
                "VERDICT    {} - {}",
                self.verdict.0.label(),
                self.verdict.1
            ),
        );
        if !self.tested_in_game {
            line(
                &mut out,
                "           UNTESTED: no Minecraft client is available to this tool. The package \
                 is \
                 checked against Horizon's documented package rules and the game's published \
                 formats, not by loading it in a running game.",
            );
        }
        line(&mut out, "");

        if let Some(outcome) = self.outcome {
            line(&mut out, "CONTENT");
            line(
                &mut out,
                &format!(
                    "  {} file(s) carried, {} converted, {} copied unchanged",
                    outcome.files.len(),
                    outcome.converted,
                    outcome.copied
                ),
            );
            if outcome.declares_content {
                line(
                    &mut out,
                    &format!(
                        "  {} block/item declaration(s) written under `data/<id>/horizon/`",
                        outcome.guessed_templates.len()
                    ),
                );
            }
            if !outcome.guessed_templates.is_empty() {
                line(
                    &mut out,
                    "  templates were inferred from blockstate properties (check them):",
                );
                for (what, template) in &outcome.guessed_templates {
                    line(&mut out, &format!("    {what} -> {template}"));
                }
            }
            line(&mut out, "");

            let errors: Vec<&Note> = outcome.unsupported().collect();
            let warnings: Vec<&Note> = outcome.warnings().collect();
            if !errors.is_empty() {
                line(&mut out, "UNSUPPORTED (these are not in the package)");
                for note in &errors {
                    let at = note
                        .file
                        .as_deref()
                        .map(|f| format!(" [{f}]"))
                        .unwrap_or_default();
                    line(&mut out, &format!("  ! {}{at}", note.message));
                    line(&mut out, &format!("      code {}", note.code));
                }
                line(&mut out, "");
            }
            if !warnings.is_empty() {
                line(&mut out, "WARNINGS");
                for note in &warnings {
                    let at = note
                        .file
                        .as_deref()
                        .map(|f| format!(" [{f}]"))
                        .unwrap_or_default();
                    line(&mut out, &format!("  * {}{at}", note.message));
                    line(&mut out, &format!("      code {}", note.code));
                }
                line(&mut out, "");
            }
        }

        if !self.detection.concerns.is_empty() {
            line(&mut out, "BEFORE YOU RELY ON THIS");
            for (code, message) in &self.detection.concerns {
                line(&mut out, &format!("  * {message}"));
                line(&mut out, &format!("      code {code}"));
            }
            line(&mut out, "");
        }

        if !self.problems.is_empty() {
            line(&mut out, "PACKAGE CHECKS");
            for problem in &self.problems {
                let at = problem
                    .path
                    .as_deref()
                    .map(|p| format!(" [{p}]"))
                    .unwrap_or_default();
                line(
                    &mut out,
                    &format!(
                        "  {} {}{at}: {}",
                        problem.severity.label(),
                        problem.code,
                        problem.message
                    ),
                );
            }
            line(&mut out, "");
        }

        if let Some((name, files, bytes)) = &self.package {
            line(&mut out, "PACKAGE");
            line(&mut out, &format!("  {name}"));
            line(
                &mut out,
                &format!("  {files} file(s), {bytes} bytes, zip with stored PNGs"),
            );
            if let Some(manifest) = self.manifest {
                line(
                    &mut out,
                    &format!(
                        "  manifest: id `{}`, api {}, side `{}`",
                        manifest.id,
                        crate::horizon::API,
                        manifest.side.as_str()
                    ),
                );
            }
            line(&mut out, "");
        } else if self.problems.iter().any(|p| p.severity == crate::validate::Severity::Error) {
            line(&mut out, "PACKAGE    not written: the checks above failed");
            line(&mut out, "");
        }

        if !self.tested_in_game {
            line(&mut out, "WHAT WAS NOT DONE");
            line(
                &mut out,
                "  - No package was installed or loaded in Minecraft: none is available here.",
            );
            line(
                &mut out,
                "  - Java code, mixins and loader behaviour were reported, never executed.",
            );
            line(
                &mut out,
                "  - Horizon content declarations use templates guessed from blockstates.",
            );
        }
        out
    }

    /// The machine-readable report: the same facts, for a build script.
    pub fn json(&self) -> hb_json::Value {
        use hb_json::Value;
        let mut root = hb_json::Object::new();
        root.insert(
            "schema",
            Value::str("zoro-horizon-bridge/report@1"),
        );
        root.insert("tool_version", Value::str(crate::VERSION));
        root.insert("jar", Value::str(self.jar_path.to_owned()));
        root.insert("jar_name", Value::str(self.jar_name.to_owned()));

        let detection = self.detection;
        root.insert(
            "source",
            Value::obj([
                (
                    "loader",
                    match &detection.loader {
                        Some(loader) => Value::str(loader.id()),
                        None => Value::str("none"),
                    },
                ),
                (
                    "mod_id",
                    detection
                        .metadata
                        .as_ref()
                        .map(|i| Value::str(i.id.clone()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "mod_version",
                    detection
                        .metadata
                        .as_ref()
                        .map(|i| Value::str(i.version.clone()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "minecraft",
                    detection
                        .version
                        .map(|v| Value::str(v.id))
                        .unwrap_or(Value::Null),
                ),
                (
                    "confidence",
                    Value::str(detection.confidence.label().to_owned()),
                ),
                (
                    "java_class_version",
                    detection
                        .class_major
                        .map(|m| Value::int(m as i64))
                        .unwrap_or(Value::Null),
                ),
                (
                    "evidence",
                    Value::arr(detection.evidence.iter().cloned().map(Value::str)),
                ),
                (
                    "concerns",
                    Value::arr(detection.concerns.iter().map(|(code, message)| {
                        Value::obj([
                            ("code", Value::str(code.clone())),
                            ("message", Value::str(message.clone())),
                        ])
                    })),
                ),
            ]),
        );

        root.insert(
            "target",
            Value::obj([
                ("minecraft", Value::str(self.target.id)),
                ("family", Value::str(self.target.family)),
                ("layout", Value::str(self.target.layout.label().to_owned())),
                (
                    "resource_format",
                    Value::str(self.target.resource_format.to_string_short()),
                ),
                (
                    "data_format",
                    Value::str(self.target.data_format.to_string_short()),
                ),
            ]),
        );

        root.insert(
            "verdict",
            Value::obj([
                ("label", Value::str(self.verdict.0.label())),
                ("explanation", Value::str(self.verdict.1.clone())),
                ("tested_in_game", Value::Bool(self.tested_in_game)),
            ]),
        );

        if let Some(outcome) = self.outcome {
            root.insert(
                "conversion",
                Value::obj([
                    ("files", Value::int(outcome.files.len() as i64)),
                    ("converted", Value::int(outcome.converted as i64)),
                    ("copied", Value::int(outcome.copied as i64)),
                    ("declares_content", Value::Bool(outcome.declares_content)),
                    (
                        "notes",
                        Value::arr(outcome.notes.iter().map(|note| {
                            Value::obj([
                                ("code", Value::str(note.code)),
                                (
                                    "severity",
                                    Value::str(match note.severity {
                                        Severity::Error => "error",
                                        Severity::Warning => "warning",
                                        Severity::Info => "info",
                                    }),
                                ),
                                ("message", Value::str(note.message.clone())),
                                (
                                    "file",
                                    note.file
                                        .as_ref()
                                        .map(|f| Value::str(f.clone()))
                                        .unwrap_or(Value::Null),
                                ),
                            ])
                        })),
                    ),
                    (
                        "unsupported",
                        Value::arr(
                            outcome
                                .unsupported()
                                .map(|note| Value::str(note.message.clone())),
                        ),
                    ),
                    (
                        "guessed_templates",
                        Value::arr(outcome.guessed_templates.iter().map(|(what, template)| {
                            Value::obj([
                                ("content", Value::str(what.clone())),
                                ("template", Value::str(template.clone())),
                            ])
                        })),
                    ),
                ]),
            );
        }

        root.insert(
            "checks",
            Value::arr(self.problems.iter().map(|problem| {
                Value::obj([
                    ("code", Value::str(problem.code)),
                    (
                        "severity",
                        Value::str(match problem.severity {
                            crate::validate::Severity::Error => "error",
                            crate::validate::Severity::Warning => "warning",
                        }),
                    ),
                    ("message", Value::str(problem.message.clone())),
                    (
                        "path",
                        problem
                            .path
                            .as_ref()
                            .map(|p| Value::str(p.clone()))
                            .unwrap_or(Value::Null),
                    ),
                ])
            })),
        );

        root.insert(
            "package",
            match &self.package {
                Some((name, files, bytes)) => Value::obj([
                    ("name", Value::str(name.clone())),
                    ("files", Value::int(*files as i64)),
                    ("bytes", Value::int(*bytes as i64)),
                    ("written", Value::Bool(true)),
                ]),
                None => Value::obj([("written", Value::Bool(false))]),
            },
        );

        root.insert(
            "limitations",
            Value::arr([
                Value::str(
                    "not installed or loaded in Minecraft: no client is available to this tool",
                ),
                Value::str("Java code, mixins and loader behaviour are reported, never executed"),
                Value::str(
                    "Horizon block/item templates are inferred from blockstate properties",
                ),
            ]),
        );
        Value::Obj(root)
    }

    /// A short line for a batch run's summary table.
    pub fn table_row(&self) -> String {
        let package = self
            .package
            .as_ref()
            .map(|(name, _, _)| name.as_str())
            .unwrap_or("-");
        format!(
            "{:<28} {:<8} {:<9} {:<10} {:<11} {}",
            truncate(self.jar_name, 28),
            self.detection
                .version
                .map(|v| v.id)
                .unwrap_or("?"),
            self.detection.loader_label(),
            self.target.id,
            self.verdict.0.label(),
            package
        )
    }
}

/// The header that goes with [`Report::table_row`].
pub fn table_header() -> String {
    format!(
        "{:<28} {:<8} {:<9} {:<10} {:<11} {}",
        "JAR", "SOURCE", "LOADER", "TARGET", "VERDICT", "PACKAGE"
    )
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{Note, Outcome};
    use crate::detect::Confidence;
    use hb_mcmeta::Loader;

    fn detection() -> Detection {
        Detection {
            loader: Some(Loader::Fabric),
            metadata: None,
            version: hb_versions::release("1.21.1"),
            versions: vec![hb_versions::release("1.21.1").unwrap()],
            confidence: Confidence::Stated,
            class_major: Some(61),
            pack_formats: None,
            evidence: vec!["`1.21.1` names exactly one release".to_owned()],
            concerns: vec![(
                "detect.version.guessed".to_owned(),
                "pass --source-version".to_owned(),
            )],
        }
    }

    fn report<'a>(
        outcome: &'a Outcome,
        detection: &'a Detection,
    ) -> Report<'a> {
        Report {
            jar_path: "mods/example.jar",
            jar_name: "example.jar",
            detection,
            verdict: (
                Verdict::Partly,
                "passive content converts; 3 class file(s) do not, so blocks will be inert".into(),
            ),
            outcome: Some(outcome),
            target: hb_versions::horizon_target(),
            manifest: None,
            problems: vec![Problem {
                code: "content.item.undeclared",
                severity: crate::validate::Severity::Warning,
                path: Some("gem item model".to_owned()),
                message: "no declaration".to_owned(),
            }],
            package: Some(("example-1.0-mc26.3.hmod".to_owned(), 12, 4096)),
            tested_in_game: false,
        }
    }

    #[test]
    fn the_text_report_leads_with_the_verdict_and_says_untested() {
        let mut outcome = Outcome::default();
        outcome.files.insert("a.json".into(), b"{}".to_vec());
        outcome
            .notes
            .push(Note::error("java.mixin", "Mixins rewrite game classes at load time"));
        let detection = detection();
        let text = report(&outcome, &detection).text();
        assert!(text.contains("VERDICT    PARTLY"));
        assert!(text.contains("UNTESTED"));
        assert!(text.contains("UNSUPPORTED"));
        assert!(text.contains("Mixins rewrite game classes"));
        assert!(text.contains("WHAT WAS NOT DONE"));
        // The guessed-source warning is not buried.
        assert!(text.contains("BEFORE YOU RELY ON THIS"));
    }

    #[test]
    fn the_json_report_carries_the_same_facts() {
        let mut outcome = Outcome::default();
        outcome
            .notes
            .push(Note::error("java.mixin", "Mixins cannot be carried"));
        let detection = detection();
        let json = report(&outcome, &detection).json();
        assert_eq!(json.get_str("schema"), Some("zoro-horizon-bridge/report@1"));
        assert_eq!(
            json.get("verdict").unwrap().get_str("label"),
            Some("PARTLY")
        );
        assert_eq!(
            json.get("verdict").unwrap().get("tested_in_game").unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            json.get("target").unwrap().get_str("minecraft"),
            Some("26.3")
        );
        let unsupported = json
            .get("conversion")
            .unwrap()
            .get("unsupported")
            .unwrap()
            .as_arr()
            .unwrap();
        assert_eq!(unsupported.len(), 1);
        assert_eq!(
            json.get("package").unwrap().get_str("name"),
            Some("example-1.0-mc26.3.hmod")
        );
    }

    #[test]
    fn exit_codes_distinguish_refused_from_built() {
        let outcome = Outcome::default();
        let detection = detection();
        let mut r = report(&outcome, &detection);
        assert_eq!(r.exit_code(), 0);
        r.package = None;
        assert_eq!(r.exit_code(), 1);
        r.problems.push(Problem {
            code: "package.java.leftover",
            severity: crate::validate::Severity::Error,
            path: None,
            message: "class file".into(),
        });
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn a_doesnt_run_verdict_always_exits_2() {
        let outcome = Outcome::default();
        let detection = detection();
        let mut r = report(&outcome, &detection);
        r.verdict = (Verdict::WontRun, "only code".into());
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn table_rows_are_aligned() {
        let outcome = Outcome::default();
        let detection = detection();
        let row = report(&outcome, &detection).table_row();
        assert!(row.starts_with("example.jar"));
        assert!(row.contains("PARTLY"));
        assert!(row.contains("26.3"));
        // The columns after the jar name line up; the name itself is as long
        // as the file's.
        let header = table_header();
        assert_eq!(
            header.find("SOURCE"),
            row.find("1.21.1"),
            "the columns must line up"
        );
        assert_eq!(header.find("PACKAGE").is_some(), true);
    }

    #[test]
    fn an_unstated_version_shows_a_question_mark_not_a_guess() {
        let mut detection = detection();
        detection.version = None;
        detection.confidence = Confidence::Unknown;
        let outcome = Outcome::default();
        let row = report(&outcome, &detection).table_row();
        // A version nobody stated is a question mark in its column, not the
        // oldest or newest release standing in for it.
        assert_eq!(&row[29..37], "?       ", "{row}");
    }
}
