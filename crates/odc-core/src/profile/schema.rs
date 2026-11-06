//! Serde model for 'odc.profile/v0' documents.
//!
//! A profile is data, not code: it names the source structures of one
//! dictionary and the L1 elements they become. The executor
//! (['super::exec']) only understands the operators declared here, so a
//! profile can never run arbitrary logic.
//!
//! Field names follow the v0 design draft except for the header block, which
//! this crate stores as 'schema_version' / 'fingerprint' / 'provenance' (the
//! draft's 'schema' / 'probe' / 'review' are accepted as aliases so draft
//! documents still load).

use std::collections::BTreeMap;

use serde::Deserialize;

/// The only profile schema this crate executes.
pub const SCHEMA_VERSION: &str = "odc.profile/v0";

/// Stage a rule belongs to.
///
/// Stages run in the declaration order below. 'Cleanup' is the exception: its
/// rules run before every other stage (design draft section 5.3), so forbidden
/// tags and attributes are gone before anything is mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Entry root, one per record.
    Root,
    /// Split a record that carries several homographs.
    Split,
    /// Cut a container into per-scope segments (pos-block).
    Segment,
    /// Map a source structure onto an L1 slot.
    Slot,
    /// Assign numbering attributes.
    Numbering,
    /// Rebuild structure without adding slots.
    Structure,
    /// Rewrite resource URLs.
    Resource,
    /// Strip forbidden tags and attributes; runs first.
    Cleanup,
}

impl Stage {
    /// Execution order used by the executor; cleanup comes first.
    pub(crate) const ORDER: [Stage; 8] = [
        Stage::Cleanup,
        Stage::Root,
        Stage::Split,
        Stage::Segment,
        Stage::Slot,
        Stage::Numbering,
        Stage::Structure,
        Stage::Resource,
    ];
}

/// One whitelisted operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Op {
    /// Re-tag a matched element, optionally replacing its children.
    Map,
    /// Wrap a matched element in a new parent.
    Wrap,
    /// Replace a matched element with its children.
    Unwrap,
    /// Change the tag name, keeping attributes and children.
    Rename,
    /// Remove a matched element, or only some of its attributes.
    Drop,
    /// Set or remove attributes.
    Attr,
    /// Move a matched element (or its children) under another element.
    Move,
    /// Split a container into one copy per matching child.
    Split,
    /// Cut sibling runs delimited by the rule boundary into containers.
    Segment,
    /// Assign a running number attribute per scope instance.
    Number,
    /// Mark an element as an alias target.
    Redirect,
}

/// Logical scope a rule is evaluated in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// The odc-entry root.
    Entry,
    /// An odc-pos-block section.
    PosBlock,
    /// An odc-sense.
    Sense,
    /// An odc-subsense.
    Subsense,
    /// An odc-sense-group.
    Group,
}

impl Scope {
    /// L1 class that materialises this scope.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Scope::Entry => "odc-entry",
            Scope::PosBlock => "odc-pos-block",
            Scope::Sense => "odc-sense",
            Scope::Subsense => "odc-subsense",
            Scope::Group => "odc-sense-group",
        }
    }
}

/// How a mapped element carries its source content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveMode {
    /// Keep the source element's children and drop the source element.
    Children,
    /// Keep the source element as the only child of the new element.
    Element,
    /// Emit an empty element; the source content is discarded.
    None,
}

/// Review state of a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleStatus {
    /// Written by hand, not reviewed yet.
    Proposed,
    /// Reviewed and accepted by a human.
    Approved,
    /// Rejected; never executed.
    Rejected,
}

/// A parsed profile.
#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    /// Schema identifier; must be 'odc.profile/v0'.
    #[serde(alias = "schema")]
    pub schema_version: String,
    /// Stable profile identifier, dot-separated.
    pub profile_id: String,
    /// Spec versions the profile was written against.
    #[serde(default)]
    pub spec: Option<SpecRef>,
    /// Dictionary identity.
    pub dict: DictInfo,
    /// Source fingerprint; compared before a profile is applied.
    #[serde(default, alias = "probe")]
    pub fingerprint: Option<Fingerprint>,
    /// How the rules were produced.
    pub engine: Engine,
    /// Ordered rule list.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Review provenance of the document.
    #[serde(default)]
    pub provenance: Option<Provenance>,
    /// Fields this crate does not interpret (coverage, resources, ...).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Spec versions a profile targets.
#[derive(Clone, Debug, Deserialize)]
pub struct SpecRef {
    /// HTML spec version.
    #[serde(default)]
    pub version: Option<String>,
    /// Slot registry version.
    #[serde(default)]
    pub slot_registry: Option<String>,
    /// Registry digest.
    #[serde(default)]
    pub registry_sha256: Option<String>,
}

/// Dictionary identity.
#[derive(Clone, Debug, Deserialize)]
pub struct DictInfo {
    /// Human-readable name.
    pub name: String,
    /// Edition, when the dictionary has one.
    #[serde(default)]
    pub edition: Option<String>,
    /// Source language.
    pub lang: String,
    /// Language variant ('en-cn' for bilingual editions).
    #[serde(default)]
    pub variant: Option<String>,
    /// Source file facts.
    #[serde(default)]
    pub source: Option<SourceInfo>,
}

/// Facts about the file the profile was inferred from.
#[derive(Clone, Debug, Deserialize)]
pub struct SourceInfo {
    /// Container format ('mdx', 'jsonl', ...).
    pub format: String,
    /// Content digest of the source file.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Number of records in the source.
    #[serde(default)]
    pub records: Option<u64>,
    /// Number of alias records.
    #[serde(default)]
    pub aliases: Option<u64>,
    /// Number of real entries.
    #[serde(default)]
    pub entries: Option<u64>,
}

/// Cheap structural fingerprint of the source markup.
#[derive(Clone, Debug, Deserialize)]
pub struct Fingerprint {
    /// Fingerprint algorithm identifier.
    pub algorithm: String,
    /// Hex digest.
    pub sha256: String,
    /// Number of samples the digest covers.
    #[serde(default)]
    pub samples: Option<u64>,
    /// Tag histogram, optional diagnostic data.
    #[serde(default)]
    pub tag_histogram: BTreeMap<String, u64>,
    /// Class histogram, optional diagnostic data.
    #[serde(default)]
    pub class_histogram: BTreeMap<String, u64>,
    /// Sample keys used while writing the profile.
    #[serde(default)]
    pub sample_ids: Vec<String>,
}

/// How the rules were produced.
#[derive(Clone, Debug, Deserialize)]
pub struct Engine {
    /// Producer name ('human' for hand-written profiles).
    pub name: String,
    /// Prompt version, for inferred profiles.
    #[serde(default)]
    pub prompt_version: Option<String>,
    /// Sampling temperature; inferred profiles must use 0.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Sampling seed.
    #[serde(default)]
    pub seed: Option<i64>,
    /// Where inference ran.
    #[serde(default)]
    pub host: Option<String>,
}

/// Review provenance of the whole document.
#[derive(Clone, Debug, Deserialize)]
pub struct Provenance {
    /// Who wrote the profile.
    #[serde(default)]
    pub source: String,
    /// Free-form note.
    #[serde(default)]
    pub note: Option<String>,
    /// Reviewer, once reviewed.
    #[serde(default)]
    pub reviewer: Option<String>,
    /// Review date, ISO 8601.
    #[serde(default)]
    pub reviewed_on: Option<String>,
}

/// One translation rule.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Globally unique, dot-separated identifier.
    pub id: String,
    /// Stage the rule runs in.
    pub stage: Stage,
    /// Higher runs first inside a stage.
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// Operator to run.
    pub op: Op,
    /// What the rule matches.
    #[serde(rename = "match")]
    pub matches: MatchSpec,
    /// What the rule emits.
    #[serde(default)]
    pub emit: EmitSpec,
    /// Sibling run delimiters, used by the segment operator.
    #[serde(default)]
    pub boundary: Option<Boundary>,
    /// Child splitter, used by the split operator.
    #[serde(default)]
    pub split_by: Option<SplitBy>,
    /// Scope instances that restart numbering, used by the number operator.
    #[serde(default)]
    pub reset_on: Vec<Scope>,
    /// Sub-rules applied inside the emitted element.
    #[serde(default)]
    pub children: Vec<Rule>,
    /// Attribute patterns to strip, used by the drop operator.
    #[serde(default)]
    pub attributes: Vec<String>,
    /// Why the rule exists, usually quoted source evidence.
    #[serde(default)]
    pub rationale: Option<String>,
    /// Confidence, 0-1; required for inferred rules.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Review state.
    #[serde(default)]
    pub status: Option<RuleStatus>,
    /// Rule-level provenance.
    #[serde(default)]
    pub provenance: Option<RuleProvenance>,
}

const fn default_priority() -> i32 {
    50
}

/// What a rule matches.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    /// CSS subset selector; a comma-separated list is allowed.
    pub selector: String,
    /// Logical scopes the match must sit inside, outermost first.
    #[serde(default)]
    pub scope: Vec<Scope>,
    /// Required text of the matched element (whitespace-normalised).
    #[serde(default)]
    pub text: Option<String>,
    /// Required attribute values.
    #[serde(default)]
    pub attr: BTreeMap<String, String>,
    /// How many elements the rule expects.
    #[serde(default)]
    pub cardinality: Option<Cardinality>,
    /// Minimum child count, used by the split operator.
    #[serde(default)]
    pub min_count: Option<usize>,
    /// Required computed display value; informational only.
    #[serde(default)]
    pub css_display: Option<String>,
}

/// Match count bounds.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cardinality {
    /// Minimum number of matches; a miss fails the rule.
    #[serde(default)]
    pub min: usize,
    /// Maximum number of matches; null means unbounded.
    #[serde(default)]
    pub max: Option<usize>,
}

/// What a rule emits.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitSpec {
    /// Tag name; defaults to the matched element's tag.
    #[serde(default, alias = "tag")]
    pub element: Option<String>,
    /// Class list written in one class attribute.
    #[serde(default, alias = "class")]
    pub classes: Vec<String>,
    /// data-* attributes; values support placeholders.
    #[serde(default)]
    pub data: BTreeMap<String, String>,
    /// Plain attributes; values support placeholders.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Attribute written by the number and split operators.
    #[serde(default)]
    pub attribute: Option<String>,
    /// Numbering options for the number and split operators.
    #[serde(default)]
    pub numbering: Option<Numbering>,
    /// Destination selector for the move operator.
    #[serde(default)]
    pub target: Option<String>,
    /// Literal value for the attr operator; placeholders allowed.
    #[serde(default)]
    pub value: Option<String>,
    /// How source content is carried over.
    #[serde(default, rename = "move")]
    pub move_mode: Option<MoveMode>,
}

/// Numbering options.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbering {
    /// First number; defaults to 1.
    #[serde(default)]
    pub start: Option<i64>,
}

/// Sibling run delimiters for the segment operator.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    /// Selector that starts a run.
    pub start: String,
    /// Selectors that end the previous run.
    #[serde(default)]
    pub until: Vec<String>,
    /// Depth below the matched container where starts are looked for.
    #[serde(default)]
    pub max_depth: Option<usize>,
}

/// Child splitter for the split operator.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitBy {
    /// Selector matching the children that become separate entries.
    pub selector: String,
    /// Keep the split child as-is instead of unwrapping it.
    #[serde(default)]
    pub keep_selector: bool,
}

/// Rule-level provenance.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleProvenance {
    /// 'human' or 'ai'.
    #[serde(default)]
    pub source: String,
    /// Model or tool name.
    #[serde(default)]
    pub model: Option<String>,
    /// Samples the rule was observed on.
    #[serde(default)]
    pub samples: Vec<String>,
    /// Free-form note.
    #[serde(default)]
    pub note: Option<String>,
    /// Why the rule is believed to be correct.
    #[serde(default)]
    pub rationale: Option<String>,
}

impl Profile {
    /// Parse a profile document.
    ///
    /// # Errors
    ///
    /// Returns ['super::ProfileError'] when the JSON is malformed, the schema
    /// identifier is unknown, or the rule list is empty.
    pub fn from_json(text: &str) -> Result<Self, super::ProfileError> {
        let profile: Self = serde_json::from_str(text).map_err(super::ProfileError::Json)?;
        if profile.schema_version != SCHEMA_VERSION {
            return Err(super::ProfileError::Schema(profile.schema_version));
        }
        if profile.rules.is_empty() {
            return Err(super::ProfileError::Empty);
        }
        Ok(profile)
    }

    /// Rules of one stage, highest priority first, declaration order on ties.
    #[must_use]
    pub fn stage_rules(&self, stage: Stage) -> Vec<&Rule> {
        let mut rules: Vec<&Rule> = self.rules.iter().filter(|r| r.stage == stage).collect();
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.priority));
        rules
    }
}
