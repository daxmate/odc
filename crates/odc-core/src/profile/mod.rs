//! Dictionary-level adaptation profiles (experimental).
//!
//! A profile is a JSON rule pack that translates one dictionary's source HTML
//! into the ODC L1 vocabulary. The design draft lives in the repository notes;
//! this module is its first executor.
//!
//! * ['schema'] parses and validates the document,
//! * ['selector'] implements the CSS subset profiles may use,
//! * ['exec'] runs the rules deterministically,
//! * ['verify'] measures text fidelity, idempotency and slot coverage.
//!
//! No rule can execute code, fetch a URL or call a model: the operator list is
//! closed and every one of them only touches the parsed tree.

pub mod contract;
pub mod exec;
pub mod schema;
pub mod selector;
pub mod verify;

use std::collections::BTreeMap;
use std::fmt;

pub use schema::{
    DictInfo, EmitSpec, Engine, Fingerprint, MatchSpec, MoveMode, Op, Profile, Rule, RuleStatus,
    Scope, Stage, SCHEMA_VERSION,
};

use crate::adapters::ParseContext;

/// Why a profile could not be loaded or applied.
#[derive(Debug)]
pub enum ProfileError {
    /// The document is not valid JSON for the schema.
    Json(serde_json::Error),
    /// The document declares a schema this crate does not implement.
    Schema(String),
    /// The document contains no rules.
    Empty,
    /// A selector is outside the supported subset.
    Selector {
        /// Rule that owns the selector.
        rule: String,
        /// Parser message.
        error: String,
    },
    /// A rule is missing a field its operator needs.
    Rule {
        /// Rule identifier.
        rule: String,
        /// What is missing.
        message: String,
    },
    /// A rule emits a P0 slot but does not declare 'match.cardinality.min'.
    MissingCardinality {
        /// Rule identifier.
        rule: String,
        /// P0 slots the rule emits.
        slots: Vec<String>,
    },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid profile JSON: {error}"),
            Self::Schema(schema) => write!(f, "unsupported profile schema: {schema}"),
            Self::Empty => write!(f, "profile contains no rules"),
            Self::Selector { rule, error } => write!(f, "rule {rule}: {error}"),
            Self::Rule { rule, message } => write!(f, "rule {rule}: {message}"),
            Self::MissingCardinality { rule, slots } => write!(
                f,
                "rule \"{rule}\" emits P0 slot(s) [{}]; declare match.cardinality.min (use 0 for optional slots)",
                slots.join(", ")
            ),
        }
    }
}

impl std::error::Error for ProfileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

/// Statistics of one profile run.
#[derive(Clone, Debug, Default)]
pub struct RunStats {
    /// Matches per rule, in rule order.
    pub hits: BTreeMap<String, usize>,
    /// Cardinality violations and other rule failures.
    pub failures: Vec<String>,
    /// P0 rules that produced no output.
    pub p0_missing: Vec<String>,
    /// Non-fatal problems, e.g. a boundary start below max_depth.
    pub warnings: Vec<String>,
    /// Rules that could not run at all.
    pub errors: Vec<String>,
    /// Presentation self-check (P8).
    pub presentation: crate::presentation::CheckReport,
}

impl RunStats {
    /// Whether a P0 rule that declared a minimum produced nothing.
    #[must_use]
    pub fn p0_failed(&self) -> bool {
        !self.p0_missing.is_empty()
    }
}

/// Result of applying a profile to one record.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// The record is an '@@@LINK=' alias; it maps to another key.
    Alias {
        /// Target key.
        target: String,
    },
    /// The fragment was translated.
    Adapted {
        /// L1 markup.
        html: String,
        /// Rule statistics.
        stats: RunStats,
    },
    /// A P0 rule failed; the fragment is kept verbatim with a fallback marker.
    Unadapted {
        /// Fallback markup.
        html: String,
        /// Rule statistics.
        stats: RunStats,
    },
}

impl Outcome {
    /// The markup this outcome produces, if any.
    #[must_use]
    pub fn html(&self) -> Option<&str> {
        match self {
            Self::Alias { .. } => None,
            Self::Adapted { html, .. } | Self::Unadapted { html, .. } => Some(html),
        }
    }

    /// Rule statistics, empty for aliases.
    #[must_use]
    pub fn stats(&self) -> Option<&RunStats> {
        match self {
            Self::Alias { .. } => None,
            Self::Adapted { stats, .. } | Self::Unadapted { stats, .. } => Some(stats),
        }
    }
}

impl Profile {
    /// Apply the profile to one source fragment.
    #[must_use]
    pub fn apply(&self, fragment: &str, ctx: &ParseContext) -> Outcome {
        if let Some(target) = alias_target(fragment) {
            return Outcome::Alias { target };
        }
        exec::apply(self, fragment, ctx)
    }

    /// Whether a corpus fingerprint matches the one recorded in the profile.
    ///
    /// 'None' means the profile carries no digest, so there is nothing to
    /// compare.
    #[must_use]
    pub fn fingerprint_matches(&self, digest: &str) -> Option<bool> {
        let fingerprint = self.fingerprint.as_ref()?;
        Some(fingerprint.sha256.eq_ignore_ascii_case(digest))
    }
}

/// Target of an '@@@LINK=' alias record, if the fragment is one.
#[must_use]
pub fn alias_target(fragment: &str) -> Option<String> {
    let trimmed = fragment.trim_start();
    let target = trimmed.strip_prefix("@@@LINK=")?;
    let target = target.lines().next().unwrap_or("").trim();
    if target.is_empty() {
        return None;
    }
    Some(String::from(target))
}
