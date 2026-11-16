//! The engine behind `hb`: inspect a Java mod jar, convert its passive content
//! for a target Minecraft version, validate the result against Horizon's
//! package rules, and write a `.hmod`.
//!
//! Modules:
//!
//! * [`jar`] reads a jar into an inspectable model (`Jar`), bounded and
//!   without guessing.
//! * [`detect`] works out which Minecraft version and loader a jar was made
//!   for, from what the jar itself says.
//! * [`convert`] rewrites assets and data for a target release's formats,
//!   recording every change and every refusal.
//! * [`horizon`] writes `horizon-mod.toml` and validates against the official
//!   manifest rules.
//! * [`validate`] checks a finished package the way Horizon's reader does.
//! * [`package`] assembles and writes the `.hmod`.
//! * [`report`] renders the human and machine-readable reports.

/// The tool's version, recorded in every package it writes.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod bridge;
pub mod convert;
pub mod detect;
pub mod horizon;
pub mod jar;
pub mod package;
pub mod report;
pub mod validate;
