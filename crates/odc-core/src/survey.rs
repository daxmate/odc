//! Dialect survey: measure a dictionary's HTML without any profile.
//!
//! The survey answers one question before anyone writes a profile: how much
//! human work would a new dialect need? It deliberately knows nothing about
//! the profile format, the rule set or the executor. It only counts what the
//! corpus itself repeats:
//!
//! * a slot kind is (tag, first class token) of one element inside an entry
//!   container;
//! * a template is the *set* of slot kinds found on an entry container's
//!   direct children plus one nesting level;
//! * a cluster is the set of entries sharing one template;
//! * an outlier is a cluster holding less than 0.5% of the scanned entries.
//!
//! The counting model is documented in docs/dialect-survey.md.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use html5ever::driver::parse_fragment_for_element;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{ns, Attribute, LocalName, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom};
use serde::Serialize;

use crate::error::{Error, Result};

/// Prefix of an MDX alias record (@@@LINK=target).
pub const ALIAS_PREFIX: &str = "@@@LINK=";

/// Default number of raw samples kept per template cluster.
pub const DEFAULT_SAMPLES_PER_CLUSTER: usize = 5;

/// Largest sample body kept on disk, in bytes.
pub const MAX_SAMPLE_BYTES: usize = 256 * 1024;

/// Total budget for kept samples, in bytes. A full-book scan can produce
/// thousands of clusters; without a cap the sample copies would dominate
/// memory. Samples stop being kept once the budget is spent.
pub const SAMPLE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

/// A cluster holding less than this share of the corpus is an outlier.
pub const OUTLIER_SHARE: f64 = 0.005;

/// Hard-coded verdict thresholds. They are constants, not flags: changing them
/// would change the ruler.
const EASY_KINDS: usize = 12;
const EASY_CLUSTERS: usize = 20;
const EASY_OUTLIERS: f64 = 0.05;
const MEDIUM_KINDS: usize = 25;
const MEDIUM_CLUSTERS: usize = 60;

/// One slot kind: a tag plus the first token of its class attribute.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotKind {
    /// Lower-case tag name.
    pub tag: String,
    /// First class token, or the empty string when the element has none.
    pub class: String,
}

impl SlotKind {
    /// Build a slot kind from a tag and an optional class attribute.
    #[must_use]
    pub fn new(tag: &str, class: Option<&str>) -> Self {
        Self {
            tag: tag.to_ascii_lowercase(),
            class: class.unwrap_or("").to_string(),
        }
    }

    /// Human readable label: tag.class, or just tag.
    #[must_use]
    pub fn label(&self) -> String {
        if self.class.is_empty() {
            self.tag.clone()
        } else {
            format!("{}.{}", self.tag, self.class)
        }
    }
}

/// How entry containers were located.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntrySource {
    /// The --entry-selector value matched the container.
    Selector,
    /// A class token named entry marked the container.
    ClassEntry,
    /// No marker was found; a repeated container was used instead.
    Fallback,
    /// No container could be identified at all.
    None,
}

impl EntrySource {
    /// Stable identifier used in JSON and in the markdown header.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Selector => "selector",
            Self::ClassEntry => "class:entry",
            Self::Fallback => "fallback",
            Self::None => "none",
        }
    }
}

/// Difficulty grade produced by the fixed thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Few kinds, few clusters, few outliers.
    Easy,
    /// Bounded growth; still writable by hand.
    Medium,
    /// Needs a human to look at the clusters.
    Manual,
}

impl Verdict {
    /// Stable identifier used in JSON and on the command line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Medium => "medium",
            Self::Manual => "manual",
        }
    }
}

/// Options for one survey run.
#[derive(Clone, Debug)]
pub struct SurveyOptions {
    /// Maximum number of records to read (None = every record).
    pub limit: Option<usize>,
    /// Optional selector overriding the entry class heuristic.
    pub entry_selector: Option<String>,
    /// Raw samples kept per cluster.
    pub samples_per_cluster: usize,
}

impl Default for SurveyOptions {
    fn default() -> Self {
        Self {
            limit: None,
            entry_selector: None,
            samples_per_cluster: DEFAULT_SAMPLES_PER_CLUSTER,
        }
    }
}

impl SurveyOptions {
    /// Options carrying only a record limit.
    #[must_use]
    pub fn with_limit(limit: Option<usize>) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }
}

/// One raw entry kept as evidence for a cluster.
#[derive(Clone, Debug, Serialize)]
pub struct SurveySample {
    /// Record key as stored in the MDX file.
    pub key: String,
    /// File name (inside the cluster directory) for this sample.
    pub file: String,
    /// Raw MDX HTML; never serialised into survey.json.
    #[serde(skip_serializing)]
    pub html: String,
}

/// Per-kind counters.
#[derive(Clone, Debug, Serialize)]
pub struct KindReport {
    /// tag.class label.
    pub kind: String,
    /// Tag name.
    pub tag: String,
    /// First class token (empty when absent).
    pub class: String,
    /// Total occurrences, counting repeats inside a folded run.
    pub occurrences: usize,
    /// Entries containing at least one occurrence.
    pub entries: usize,
    /// entries / entries_scanned.
    pub presence: f64,
}

/// One template cluster.
#[derive(Clone, Debug, Serialize)]
pub struct ClusterReport {
    /// Stable id (c001...) ordered by cluster size.
    pub id: String,
    /// Number of entries in this cluster.
    pub entries: usize,
    /// entries / entries_scanned.
    pub share: f64,
    /// True when the cluster is below OUTLIER_SHARE.
    pub outlier: bool,
    /// Sorted slot kinds forming the template.
    pub kinds: Vec<String>,
    /// Layout of the first sample (top-level slots with their children).
    pub layout: String,
    /// Up to samples_per_cluster evidence entries.
    pub samples: Vec<SurveySample>,
}

/// One histogram bucket.
#[derive(Clone, Debug, Serialize)]
pub struct Bucket {
    /// Bucket label.
    pub label: String,
    /// Number of observations.
    pub count: usize,
}

/// A kind that appears in some entries but not all of them.
#[derive(Clone, Debug, Serialize)]
pub struct OptionalKind {
    /// tag.class label.
    pub kind: String,
    /// entries / entries_scanned.
    pub presence: f64,
}

/// Structural statistics collected while scanning.
#[derive(Clone, Debug, Serialize)]
pub struct StatsReport {
    /// Share of entries held by the largest cluster.
    pub top_cluster_share: f64,
    /// Clusters needed to cover 80% of the entries.
    pub clusters_for_80: usize,
    /// Clusters needed to cover 90% of the entries.
    pub clusters_for_90: usize,
    /// Deepest element nesting seen inside an entry container.
    pub max_depth: usize,
    /// Mean element nesting depth.
    pub mean_depth: f64,
    /// Distribution of the deepest nesting per entry.
    pub depth: Vec<Bucket>,
    /// Distribution of folded sibling runs.
    pub runs: Vec<Bucket>,
    /// Mean number of distinct slot kinds per entry.
    pub mean_kinds: f64,
    /// Kinds that do not appear on every entry (presence below 1).
    pub optional: Vec<OptionalKind>,
}

/// The fixed verdict plus the numbers behind it.
#[derive(Clone, Debug, Serialize)]
pub struct VerdictReport {
    /// Grade.
    pub grade: Verdict,
    /// Distinct slot kinds seen.
    pub kinds: usize,
    /// Distinct templates seen.
    pub clusters: usize,
    /// Share of entries living in outlier clusters.
    pub outlier_entry_share: f64,
    /// Share of clusters that are outliers.
    pub outlier_cluster_share: f64,
    /// Human readable explanation of the grade.
    pub reasons: Vec<String>,
}

/// Machine readable result of one survey.
#[derive(Clone, Debug, Serialize)]
pub struct SurveyReport {
    /// Tool name.
    pub tool: &'static str,
    /// Report schema version.
    pub schema: u32,
    /// Limit that was applied, if any.
    pub limit: Option<usize>,
    /// Selector that was supplied, if any.
    pub entry_selector: Option<String>,
    /// How entry containers were located.
    pub entry_source: EntrySource,
    /// Records read from the file.
    pub records: usize,
    /// Entry units analysed (a record may hold several).
    pub entries_scanned: usize,
    /// Alias records (@@@LINK) skipped.
    pub alias: usize,
    /// Empty records skipped.
    pub empty: usize,
    /// Records without a usable entry container.
    pub unparsed: usize,
    /// Records that held more than one entry container.
    pub multi_entry_records: usize,
    /// Slot kind table, ordered by occurrences.
    pub kinds: Vec<KindReport>,
    /// Template clusters, ordered by size.
    pub clusters: Vec<ClusterReport>,
    /// Structural statistics.
    pub stats: StatsReport,
    /// Difficulty verdict.
    pub verdict: VerdictReport,
}

impl SurveyReport {
    /// Render the human readable survey.md body.
    #[must_use]
    pub fn to_markdown(&self, source: &str, elapsed_ms: u128) -> String {
        let mut out = String::new();
        out.push_str("# Dictionary survey\n\n");
        out.push_str(&format!("- source: {source}\n"));
        out.push_str(&format!(
            "- limit: {}\n",
            self.limit
                .map_or_else(|| "full".to_string(), |v| v.to_string())
        ));
        out.push_str(&format!(
            "- records: {} (entries {} / alias {} / empty {} / unparsed {})\n",
            self.records, self.entries_scanned, self.alias, self.empty, self.unparsed
        ));
        out.push_str(&format!(
            "- entry containers: {} ({} record(s) held more than one)\n",
            self.entry_source.as_str(),
            self.multi_entry_records
        ));
        out.push_str(&format!("- elapsed: {elapsed_ms} ms\n\n"));

        out.push_str(&format!(
            "## Verdict: {}\n\n",
            self.verdict.grade.as_str().to_uppercase()
        ));
        for reason in &self.verdict.reasons {
            out.push_str(&format!("- {reason}\n"));
        }
        out.push('\n');

        out.push_str(&format!("## Slot kinds ({})\n\n", self.kinds.len()));
        out.push_str("| # | kind | occurrences | entries | presence |\n");
        out.push_str("|---|------|-------------|---------|----------|\n");
        for (index, kind) in self.kinds.iter().enumerate() {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.1}% |\n",
                index + 1,
                kind.kind,
                kind.occurrences,
                kind.entries,
                kind.presence * 100.0
            ));
        }
        out.push('\n');

        out.push_str(&format!(
            "## Template clusters ({})\n\n",
            self.clusters.len()
        ));
        out.push_str("| id | entries | share | outlier | kinds |\n");
        out.push_str("|----|---------|-------|---------|-------|\n");
        for cluster in &self.clusters {
            out.push_str(&format!(
                "| {} | {} | {:.2}% | {} | {} |\n",
                cluster.id,
                cluster.entries,
                cluster.share * 100.0,
                if cluster.outlier { "yes" } else { "no" },
                cluster.kinds.join(" + ")
            ));
        }
        out.push('\n');

        // A full-book scan can produce thousands of singleton clusters. Keep
        // the table complete (it is the census) but expand only the largest
        // clusters; the rest stay in survey.json and samples/.
        let detailed = self.clusters.len().min(200);
        for cluster in self.clusters.iter().take(detailed) {
            out.push_str(&format!(
                "### {} ({} entries, {:.2}%)\n",
                cluster.id,
                cluster.entries,
                cluster.share * 100.0
            ));
            if !cluster.layout.is_empty() {
                out.push_str(&format!("layout: {}\n", cluster.layout));
            }
            let keys: Vec<&str> = cluster
                .samples
                .iter()
                .take(3)
                .map(|sample| sample.key.as_str())
                .collect();
            if !keys.is_empty() {
                out.push_str(&format!("samples: {}\n", keys.join(", ")));
            }
            out.push('\n');
        }
        if self.clusters.len() > detailed {
            out.push_str(&format!(
                "Cluster details are expanded for the {detailed} largest clusters; the remaining {} are listed in survey.json and samples/ only.\n\n",
                self.clusters.len() - detailed
            ));
        }

        out.push_str("## Structure\n\n");
        out.push_str(&format!(
            "- largest cluster: {:.1}% of entries\n",
            self.stats.top_cluster_share * 100.0
        ));
        out.push_str(&format!(
            "- clusters needed to cover 80% / 90% of entries: {} / {}\n",
            self.stats.clusters_for_80, self.stats.clusters_for_90
        ));
        out.push_str(&format!(
            "- depth: max {}, mean {:.2}\n",
            self.stats.max_depth, self.stats.mean_depth
        ));
        for bucket in &self.stats.depth {
            out.push_str(&format!("  - depth {}: {}\n", bucket.label, bucket.count));
        }
        out.push_str(&format!(
            "- mean distinct kinds per entry: {:.2}\n",
            self.stats.mean_kinds
        ));
        out.push_str("- folded sibling runs:\n");
        for bucket in &self.stats.runs {
            out.push_str(&format!("  - run {}: {}\n", bucket.label, bucket.count));
        }
        if self.stats.optional.is_empty() {
            out.push_str("- optional blocks: none\n");
        } else {
            out.push_str("- optional blocks (presence below 100%):\n");
            for kind in self.stats.optional.iter().take(20) {
                out.push_str(&format!(
                    "  - {}: {:.1}%\n",
                    kind.kind,
                    kind.presence * 100.0
                ));
            }
            if self.stats.optional.len() > 20 {
                out.push_str(&format!(
                    "  - ... {} more (see survey.json)\n",
                    self.stats.optional.len() - 20
                ));
            }
        }
        out
    }
}

/// Accumulates records into a SurveyReport.
pub struct Surveyor {
    options: SurveyOptions,
    selector: Option<Vec<CompoundSelector>>,
    class_hits: usize,
    fallback_hits: usize,
    records: usize,
    entries_scanned: usize,
    alias: usize,
    empty: usize,
    unparsed: usize,
    multi_entry_records: usize,
    kinds: BTreeMap<SlotKind, KindCounter>,
    clusters: BTreeMap<Vec<SlotKind>, ClusterCounter>,
    depth_hist: [usize; 11],
    runs_hist: [usize; 5],
    depth_sum: usize,
    kinds_per_entry_sum: usize,
    sample_bytes: usize,
}

#[derive(Default)]
struct KindCounter {
    occurrences: usize,
    entries: usize,
}

struct ClusterCounter {
    entries: usize,
    layout: String,
    samples: Vec<SurveySample>,
}

/// Slots and shape of one entry container.
struct EntryProfile {
    slots: BTreeSet<SlotKind>,
    occurrences: Vec<(SlotKind, usize)>,
    runs: Vec<usize>,
    depth: usize,
    layout: String,
    empty: bool,
}

impl Surveyor {
    /// Start a survey.
    ///
    /// # Errors
    ///
    /// Fails with Error::InvalidArgument when the selector cannot be parsed.
    pub fn new(options: SurveyOptions) -> Result<Self> {
        let selector = match &options.entry_selector {
            Some(text) => Some(parse_selector(text)?),
            None => None,
        };
        Ok(Self {
            options,
            selector,
            class_hits: 0,
            fallback_hits: 0,
            records: 0,
            entries_scanned: 0,
            alias: 0,
            empty: 0,
            unparsed: 0,
            multi_entry_records: 0,
            kinds: BTreeMap::new(),
            clusters: BTreeMap::new(),
            depth_hist: [0; 11],
            runs_hist: [0; 5],
            depth_sum: 0,
            kinds_per_entry_sum: 0,
            sample_bytes: 0,
        })
    }

    /// Number of records read so far.
    #[must_use]
    pub fn records(&self) -> usize {
        self.records
    }

    /// Number of entry units analysed so far.
    #[must_use]
    pub fn entries_scanned(&self) -> usize {
        self.entries_scanned
    }

    /// Feed one MDX record. Returns false when the configured limit is already
    /// reached, so callers can stop reading the file.
    pub fn push(&mut self, key: &str, html: &str) -> bool {
        if let Some(limit) = self.options.limit {
            if self.records >= limit {
                return false;
            }
        }
        self.records += 1;

        let trimmed = html.trim();
        if trimmed.is_empty() {
            self.empty += 1;
            return true;
        }
        if trimmed.starts_with(ALIAS_PREFIX) {
            self.alias += 1;
            return true;
        }

        let dom = parse_dom(html);
        let (roots, fallback) = entry_roots(&dom, self.selector.as_deref());
        if roots.is_empty() {
            self.unparsed += 1;
            return true;
        }
        if fallback {
            self.fallback_hits += 1;
        } else if self.selector.is_none() {
            self.class_hits += 1;
        }
        if roots.len() > 1 {
            self.multi_entry_records += 1;
        }
        for root in &roots {
            self.absorb(root, key, html);
        }
        true
    }

    fn absorb(&mut self, root: &Handle, key: &str, html: &str) {
        let profile = profile_entry(root);
        if profile.empty {
            self.unparsed += 1;
            return;
        }
        self.entries_scanned += 1;
        self.depth_sum += profile.depth;
        self.kinds_per_entry_sum += profile.slots.len();
        self.depth_hist[profile.depth.min(10)] += 1;
        for run in &profile.runs {
            self.runs_hist[run_bucket(*run)] += 1;
        }
        for (kind, repeats) in &profile.occurrences {
            self.kinds.entry(kind.clone()).or_default().occurrences += repeats;
        }
        for kind in &profile.slots {
            if let Some(counter) = self.kinds.get_mut(kind) {
                counter.entries += 1;
            }
        }

        let template: Vec<SlotKind> = profile.slots.iter().cloned().collect();
        let counter = self
            .clusters
            .entry(template)
            .or_insert_with(|| ClusterCounter {
                entries: 0,
                layout: profile.layout.clone(),
                samples: Vec::new(),
            });
        counter.entries += 1;
        if counter.samples.len() < self.options.samples_per_cluster
            && self.sample_bytes < SAMPLE_BUDGET_BYTES
        {
            let file = unique_sample_file(&counter.samples, key);
            let body = truncate_sample(html);
            self.sample_bytes += body.len();
            counter.samples.push(SurveySample {
                key: key.to_string(),
                file,
                html: body,
            });
        }
    }

    /// Consume the surveyor and build the report.
    #[must_use]
    pub fn finish(self) -> SurveyReport {
        let scanned = self.entries_scanned;
        let entry_source = match (self.selector.is_some(), self.class_hits, self.fallback_hits) {
            (true, _, _) => EntrySource::Selector,
            (false, hits, fallback) if hits > 0 && hits >= fallback => EntrySource::ClassEntry,
            (false, _, fallback) if fallback > 0 => EntrySource::Fallback,
            _ => EntrySource::None,
        };

        let mut clusters: Vec<(Vec<SlotKind>, ClusterCounter)> =
            self.clusters.into_iter().collect();
        clusters.sort_by(|a, b| b.1.entries.cmp(&a.1.entries).then_with(|| a.0.cmp(&b.0)));

        let total_clusters = clusters.len();
        let mut reports = Vec::with_capacity(total_clusters);
        let mut outlier_entries = 0usize;
        let mut outlier_clusters = 0usize;
        for (index, (template, counter)) in clusters.into_iter().enumerate() {
            let share = ratio(counter.entries, scanned);
            let outlier = share < OUTLIER_SHARE;
            if outlier {
                outlier_clusters += 1;
                outlier_entries += counter.entries;
            }
            reports.push(ClusterReport {
                id: format!("c{:03}", index + 1),
                entries: counter.entries,
                share,
                outlier,
                kinds: template.iter().map(SlotKind::label).collect(),
                layout: counter.layout,
                samples: counter.samples,
            });
        }

        let mut kinds: Vec<KindReport> = self
            .kinds
            .iter()
            .map(|(kind, counter)| KindReport {
                kind: kind.label(),
                tag: kind.tag.clone(),
                class: kind.class.clone(),
                occurrences: counter.occurrences,
                entries: counter.entries,
                presence: ratio(counter.entries, scanned),
            })
            .collect();
        kinds.sort_by(|a, b| {
            b.occurrences
                .cmp(&a.occurrences)
                .then_with(|| a.kind.cmp(&b.kind))
        });

        let mut optional: Vec<OptionalKind> = kinds
            .iter()
            .filter(|kind| kind.entries < scanned)
            .map(|kind| OptionalKind {
                kind: kind.kind.clone(),
                presence: kind.presence,
            })
            .collect();
        optional.sort_by(|a, b| {
            a.presence
                .partial_cmp(&b.presence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.kind.cmp(&b.kind))
        });

        let outlier_entry_share = ratio(outlier_entries, scanned);
        let outlier_cluster_share = ratio(outlier_clusters, total_clusters);
        let kind_count = kinds.len();
        let cluster_count = reports.len();
        let grade = if scanned == 0 {
            // Nothing was measured; call it manual rather than easy so an empty
            // run never reads as a pass.
            Verdict::Manual
        } else if kind_count <= EASY_KINDS
            && cluster_count <= EASY_CLUSTERS
            && outlier_entry_share <= EASY_OUTLIERS
        {
            Verdict::Easy
        } else if kind_count <= MEDIUM_KINDS && cluster_count <= MEDIUM_CLUSTERS {
            Verdict::Medium
        } else {
            Verdict::Manual
        };
        let mut reasons = vec![
            format!(
                "slot kinds: {kind_count} (easy <= {EASY_KINDS}, medium <= {MEDIUM_KINDS})"
            ),
            format!(
                "template clusters: {cluster_count} (easy <= {EASY_CLUSTERS}, medium <= {MEDIUM_CLUSTERS})"
            ),
            format!(
                "outlier entry share: {:.2}% (easy <= {:.0}%)",
                outlier_entry_share * 100.0,
                EASY_OUTLIERS * 100.0
            ),
            format!(
                "outlier clusters: {outlier_clusters} of {total_clusters} ({:.2}%)",
                outlier_cluster_share * 100.0
            ),
        ];
        if scanned == 0 {
            reasons.insert(0, "no entry was measured".to_string());
        }

        let max_depth = self
            .depth_hist
            .iter()
            .rposition(|count| *count > 0)
            .unwrap_or(0);
        let depth: Vec<Bucket> = self
            .depth_hist
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .map(|(index, count)| Bucket {
                label: if index == 10 {
                    "10+".to_string()
                } else {
                    index.to_string()
                },
                count: *count,
            })
            .collect();
        let run_labels = ["1", "2", "3", "4-9", "10+"];
        let runs: Vec<Bucket> = self
            .runs_hist
            .iter()
            .zip(run_labels.iter())
            .map(|(count, label)| Bucket {
                label: (*label).to_string(),
                count: *count,
            })
            .collect();

        let mut cumulative = 0usize;
        let mut clusters_for_80 = 0usize;
        let mut clusters_for_90 = 0usize;
        for (index, cluster) in reports.iter().enumerate() {
            cumulative += cluster.entries;
            if clusters_for_80 == 0 && cumulative as f64 >= 0.8 * scanned as f64 {
                clusters_for_80 = index + 1;
            }
            if clusters_for_90 == 0 && cumulative as f64 >= 0.9 * scanned as f64 {
                clusters_for_90 = index + 1;
                break;
            }
        }

        let top_cluster_share = reports.first().map_or(0.0, |cluster| cluster.share);

        SurveyReport {
            tool: "odc survey",
            schema: 1,
            limit: self.options.limit,
            entry_selector: self.options.entry_selector,
            entry_source,
            records: self.records,
            entries_scanned: scanned,
            alias: self.alias,
            empty: self.empty,
            unparsed: self.unparsed,
            multi_entry_records: self.multi_entry_records,
            kinds,
            clusters: reports,
            stats: StatsReport {
                top_cluster_share,
                clusters_for_80,
                clusters_for_90,
                max_depth,
                mean_depth: ratio(self.depth_sum, scanned),
                depth,
                runs,
                mean_kinds: ratio(self.kinds_per_entry_sum, scanned),
                optional,
            },
            verdict: VerdictReport {
                grade,
                kinds: kind_count,
                clusters: cluster_count,
                outlier_entry_share,
                outlier_cluster_share,
                reasons,
            },
        }
    }
}

fn ratio(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

fn run_bucket(len: usize) -> usize {
    match len {
        1 => 0,
        2 => 1,
        3 => 2,
        4..=9 => 3,
        _ => 4,
    }
}

fn parse_dom(input: &str) -> RcDom {
    let context = Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from("div")),
        attrs: RefCell::new(Vec::<Attribute>::new()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    parse_fragment_for_element(RcDom::default(), ParseOpts::default(), context, true, None)
        .one(StrTendril::from(input))
}

fn is_element(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

fn element_tag(node: &Handle) -> String {
    match &node.data {
        NodeData::Element { name, .. } => name.local.to_string(),
        _ => String::new(),
    }
}

fn attr_value(node: &Handle, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|attr| attr.value.to_string())
}

fn class_tokens(node: &Handle) -> Vec<String> {
    attr_value(node, "class")
        .map(|value| value.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn element_children(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|child| is_element(child))
        .cloned()
        .collect()
}

fn kind_of(node: &Handle) -> SlotKind {
    let tokens = class_tokens(node);
    SlotKind::new(&element_tag(node), tokens.first().map(String::as_str))
}

fn collect_elements(node: &Handle, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if is_element(child) {
            out.push(child.clone());
            collect_elements(child, out);
        } else {
            collect_elements(child, out);
        }
    }
}

fn signature(node: &Handle) -> String {
    let tokens = class_tokens(node);
    format!(
        "{}|{}",
        element_tag(node),
        tokens.first().map_or("", String::as_str)
    )
}

fn text_len(node: &Handle) -> usize {
    let mut total = 0;
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => total += contents.borrow().chars().count(),
            NodeData::Element { .. } => total += text_len(child),
            _ => {}
        }
    }
    total
}

fn has_heading(node: &Handle) -> bool {
    for child in node.children.borrow().iter() {
        if is_element(child) {
            let tag = element_tag(child);
            if tag.len() == 2 && tag.starts_with('h') && tag.as_bytes()[1].is_ascii_digit() {
                return true;
            }
            if has_heading(child) {
                return true;
            }
        }
    }
    false
}

fn element_depth(node: &Handle) -> usize {
    let mut max = 0;
    for child in node.children.borrow().iter() {
        if is_element(child) {
            max = max.max(1 + element_depth(child));
        }
    }
    max
}

fn fold_siblings(children: &[Handle]) -> Vec<(SlotKind, usize)> {
    let mut runs: Vec<(SlotKind, usize)> = Vec::new();
    for child in children {
        let kind = kind_of(child);
        match runs.last_mut() {
            Some((last, count)) if *last == kind => *count += 1,
            _ => runs.push((kind, 1)),
        }
    }
    runs
}

fn profile_entry(root: &Handle) -> EntryProfile {
    let top = element_children(root);
    if top.is_empty() {
        return EntryProfile {
            slots: BTreeSet::new(),
            occurrences: Vec::new(),
            runs: Vec::new(),
            depth: 0,
            layout: String::new(),
            empty: true,
        };
    }

    let mut slots = BTreeSet::new();
    let mut occurrences: Vec<(SlotKind, usize)> = Vec::new();
    let mut runs: Vec<usize> = Vec::new();
    let mut layout_parts: Vec<String> = Vec::new();

    let top_runs = fold_siblings(&top);
    for (kind, count) in &top_runs {
        slots.insert(kind.clone());
        occurrences.push((kind.clone(), *count));
        runs.push(*count);
    }
    for child in &top {
        let nested = element_children(child);
        let nested_runs = fold_siblings(&nested);
        for (kind, count) in &nested_runs {
            slots.insert(kind.clone());
            occurrences.push((kind.clone(), *count));
            runs.push(*count);
        }
        if layout_parts.len() < 12 {
            let children: Vec<String> = nested_runs
                .iter()
                .map(|(kind, count)| {
                    if *count > 1 {
                        format!("{}x{}", kind.label(), count)
                    } else {
                        kind.label()
                    }
                })
                .collect();
            layout_parts.push(format!(
                "{}[{}]",
                kind_of(child).label(),
                children.join(" + ")
            ));
        }
    }

    EntryProfile {
        slots,
        occurrences,
        runs,
        depth: element_depth(root),
        layout: layout_parts.join(" "),
        empty: false,
    }
}

fn collect_roots(
    node: &Handle,
    selector: Option<&[CompoundSelector]>,
    shadowed: bool,
    out: &mut Vec<Handle>,
) {
    for child in node.children.borrow().iter() {
        if is_element(child) {
            let hit = is_root(child, selector);
            if hit && !shadowed {
                out.push(child.clone());
            }
            collect_roots(child, selector, shadowed || hit, out);
        } else {
            collect_roots(child, selector, shadowed, out);
        }
    }
}

fn is_root(node: &Handle, selector: Option<&[CompoundSelector]>) -> bool {
    match selector {
        Some(list) => list.iter().any(|candidate| candidate.matches(node)),
        None => class_tokens(node)
            .iter()
            .any(|token| token.eq_ignore_ascii_case("entry")),
    }
}

fn entry_roots(dom: &RcDom, selector: Option<&[CompoundSelector]>) -> (Vec<Handle>, bool) {
    let mut out = Vec::new();
    for child in dom.document.children.borrow().iter() {
        collect_roots(child, selector, false, &mut out);
    }
    if !out.is_empty() {
        return (out, false);
    }
    (fallback_roots(dom), true)
}

fn fallback_roots(dom: &RcDom) -> Vec<Handle> {
    let mut elements = Vec::new();
    collect_elements(&dom.document, &mut elements);
    if elements.is_empty() {
        return Vec::new();
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for element in &elements {
        *counts.entry(signature(element)).or_default() += 1;
    }

    let mut best: Option<(bool, usize, Handle)> = None;
    for element in &elements {
        if counts.get(&signature(element)).copied().unwrap_or(0) < 2 {
            continue;
        }
        if element_children(element).is_empty() {
            continue;
        }
        let candidate = (has_heading(element), text_len(element));
        let better = match &best {
            None => true,
            Some((heading, text, _)) => candidate > (*heading, *text),
        };
        if better {
            best = Some((candidate.0, candidate.1, element.clone()));
        }
    }
    if let Some((_, _, handle)) = best {
        return vec![handle];
    }

    // Last resort: the widest container that still has element children.
    let mut widest: Option<(usize, Handle)> = None;
    for element in &elements {
        if element_children(element).len() < 2 {
            continue;
        }
        let size = text_len(element);
        if widest.as_ref().is_none_or(|(best, _)| size > *best) {
            widest = Some((size, element.clone()));
        }
    }
    widest.map(|(_, handle)| vec![handle]).unwrap_or_default()
}

/// A player in the selector subset accepted by --entry-selector.
#[derive(Debug, Default)]
struct CompoundSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

impl CompoundSelector {
    fn matches(&self, node: &Handle) -> bool {
        if let Some(tag) = &self.tag {
            if !element_tag(node).eq_ignore_ascii_case(tag) {
                return false;
            }
        }
        if let Some(id) = &self.id {
            if attr_value(node, "id").as_deref() != Some(id.as_str()) {
                return false;
            }
        }
        let tokens = class_tokens(node);
        for class in &self.classes {
            if !tokens.iter().any(|token| token == class) {
                return false;
            }
        }
        for (name, value) in &self.attrs {
            let Some(actual) = attr_value(node, name) else {
                return false;
            };
            if let Some(want) = value {
                if &actual != want {
                    return false;
                }
            }
        }
        true
    }
}

fn parse_selector(input: &str) -> Result<Vec<CompoundSelector>> {
    let mut out = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(Error::InvalidArgument(format!(
                "entry selector contains an empty alternative: {input:?}"
            )));
        }
        out.push(parse_compound(part)?);
    }
    if out.is_empty() {
        return Err(Error::InvalidArgument(
            "entry selector is empty".to_string(),
        ));
    }
    Ok(out)
}

fn parse_compound(text: &str) -> Result<CompoundSelector> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut tag = String::new();
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'-') {
        tag.push(bytes[index] as char);
        index += 1;
    }
    let mut selector = CompoundSelector {
        tag: if tag.is_empty() {
            None
        } else {
            Some(tag.to_ascii_lowercase())
        },
        ..CompoundSelector::default()
    };
    let mut any = selector.tag.is_some();
    while index < bytes.len() {
        match bytes[index] {
            b'.' => {
                let (name, next) = read_ident(text, index + 1)?;
                selector.classes.push(name);
                index = next;
                any = true;
            }
            b'#' => {
                let (name, next) = read_ident(text, index + 1)?;
                selector.id = Some(name);
                index = next;
                any = true;
            }
            b'[' => {
                let rest = &text[index + 1..];
                let Some(close) = rest.find(']') else {
                    return Err(Error::InvalidArgument(format!(
                        "unterminated attribute selector in {text:?}"
                    )));
                };
                let body = &rest[..close];
                index += close + 2;
                let (name, value) = match body.split_once('=') {
                    Some((name, value)) => (
                        name.trim(),
                        Some(
                            value
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string(),
                        ),
                    ),
                    None => (body.trim(), None),
                };
                if name.is_empty() {
                    return Err(Error::InvalidArgument(format!(
                        "attribute selector without a name in {text:?}"
                    )));
                }
                selector.attrs.push((name.to_ascii_lowercase(), value));
                any = true;
            }
            other => {
                return Err(Error::InvalidArgument(format!(
                    "unsupported selector syntax {:?} in {text:?}: only tag, .class, #id and [attr] are accepted",
                    other as char
                )));
            }
        }
    }
    if !any {
        return Err(Error::InvalidArgument(format!(
            "empty selector in {text:?}"
        )));
    }
    Ok(selector)
}

fn read_ident(text: &str, start: usize) -> Result<(String, usize)> {
    let rest = &text[start..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .unwrap_or(rest.len());
    if end == 0 {
        return Err(Error::InvalidArgument(format!(
            "missing name in selector {text:?}"
        )));
    }
    Ok((rest[..end].to_string(), start + end))
}

/// File name used for one raw sample (safe-key.html).
#[must_use]
pub fn sample_file_name(key: &str) -> String {
    let mut out = String::new();
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 60 {
            break;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "entry.html".to_string()
    } else {
        format!("{trimmed}.html")
    }
}

fn unique_sample_file(existing: &[SurveySample], key: &str) -> String {
    let base = sample_file_name(key);
    if !existing.iter().any(|sample| sample.file == base) {
        return base;
    }
    let stem = base.trim_end_matches(".html").to_string();
    let mut suffix = 2;
    loop {
        let candidate = format!("{stem}-{suffix}.html");
        if !existing.iter().any(|sample| sample.file == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn truncate_sample(html: &str) -> String {
    if html.len() <= MAX_SAMPLE_BYTES {
        return html.to_string();
    }
    let mut end = MAX_SAMPLE_BYTES;
    while end > 0 && !html.is_char_boundary(end) {
        end -= 1;
    }
    html[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY_A: &str = r#"<div class="entry"><span class="a"></span><span class="a"></span><span class="b"></span></div>"#;

    fn survey(htmls: &[&str]) -> SurveyReport {
        let mut surveyor = Surveyor::new(SurveyOptions::default()).expect("options");
        for (index, html) in htmls.iter().enumerate() {
            surveyor.push(&format!("k{index}"), html);
        }
        surveyor.finish()
    }

    fn entry(body: &str) -> String {
        format!(r#"<div class="entry">{body}</div>"#)
    }

    #[test]
    fn folds_consecutive_siblings() {
        let report = survey(&[ENTRY_A]);
        let kind = report
            .kinds
            .iter()
            .find(|kind| kind.kind == "span.a")
            .expect("span.a measured");
        assert_eq!(kind.occurrences, 2);
        assert_eq!(kind.entries, 1);
        assert_eq!(report.kinds.len(), 2);
        assert_eq!(report.entries_scanned, 1);
    }

    #[test]
    fn template_is_a_set_not_a_sequence() {
        let first = entry(r#"<span class="a"></span><span class="b"></span>"#);
        let second = entry(r#"<span class="b"></span><span class="a"></span>"#);
        let report = survey(&[&first, &second]);
        assert_eq!(report.clusters.len(), 1);
        assert_eq!(report.clusters[0].entries, 2);
    }

    #[test]
    fn rare_cluster_is_an_outlier() {
        let common = entry(r#"<p class="x"></p>"#);
        let rare = entry(r#"<p class="x"></p><p class="rare"></p>"#);
        let mut htmls = vec![common.as_str(); 200];
        htmls.push(&rare);
        let report = survey(&htmls);
        assert_eq!(report.entries_scanned, 201);
        let outlier = report
            .clusters
            .iter()
            .find(|cluster| cluster.entries == 1)
            .expect("outlier cluster");
        assert!(outlier.outlier);
        assert_eq!(report.verdict.grade, Verdict::Easy);
    }

    #[test]
    fn coverage_counts_the_clusters_that_carry_the_corpus() {
        let common = entry(r#"<p class="x"></p>"#);
        let rare = entry(r#"<p class="x"></p><p class="y"></p>"#);
        let mut htmls = vec![common.as_str(); 9];
        htmls.push(&rare);
        let report = survey(&htmls);
        assert_eq!(report.stats.clusters_for_80, 1);
        assert_eq!(report.stats.clusters_for_90, 1);
        assert!((report.stats.top_cluster_share - 0.9).abs() < 1e-9);
    }

    #[test]
    fn alias_records_are_counted_apart() {
        let mut surveyor = Surveyor::new(SurveyOptions::default()).expect("options");
        surveyor.push("abandon", "@@@LINK=abandon-1");
        let report = surveyor.finish();
        assert_eq!(report.alias, 1);
        assert_eq!(report.records, 1);
        assert_eq!(report.entries_scanned, 0);
        assert!(report.clusters.is_empty());
        assert_eq!(report.entry_source, EntrySource::None);
        assert_eq!(report.verdict.grade, Verdict::Manual);
    }

    #[test]
    fn empty_records_are_counted_apart() {
        let mut surveyor = Surveyor::new(SurveyOptions::default()).expect("options");
        surveyor.push("blank", "   ");
        let report = surveyor.finish();
        assert_eq!(report.empty, 1);
        assert_eq!(report.records, 1);
        assert_eq!(report.unparsed, 0);
    }

    #[test]
    fn selector_overrides_the_class_heuristic() {
        let html = r#"<div class="entry"><p class="noise"></p></div><section id="main"><p class="a"></p><p class="b"></p></section>"#;
        let mut surveyor = Surveyor::new(SurveyOptions {
            entry_selector: Some("#main".to_string()),
            ..SurveyOptions::default()
        })
        .expect("selector parses");
        surveyor.push("k", html);
        let report = surveyor.finish();
        assert_eq!(report.entry_source, EntrySource::Selector);
        assert_eq!(report.entries_scanned, 1);
        assert_eq!(report.kinds.len(), 2);
        assert!(report.kinds.iter().all(|kind| kind.kind.starts_with("p.")));
    }

    #[test]
    fn selector_with_unsupported_syntax_is_rejected() {
        let error = Surveyor::new(SurveyOptions {
            entry_selector: Some("div > p".to_string()),
            ..SurveyOptions::default()
        });
        assert!(error.is_err());
    }

    #[test]
    fn fallback_finds_a_repeated_container_with_a_heading() {
        let html = r#"<div class="rec"><h1>one</h1><p class="x">alpha</p></div><div class="rec"><h1>two</h1><p class="x">beta</p></div>"#;
        let report = survey(&[html]);
        assert_eq!(report.entry_source, EntrySource::Fallback);
        assert_eq!(report.entries_scanned, 1);
        assert_eq!(report.kinds.len(), 2);
    }

    #[test]
    fn malformed_html_is_repaired_not_fatal() {
        let html = r#"<div class="entry"><p>alpha<div class="sense"><span class="z">"#;
        let report = survey(&[html]);
        assert_eq!(report.entries_scanned, 1);
        assert_eq!(report.unparsed, 0);
        assert!(!report.kinds.is_empty());
    }

    #[test]
    fn record_with_two_containers_measures_both() {
        let html = format!(
            "{}{}",
            entry(r#"<p class="a"></p>"#),
            entry(r#"<p class="b"></p>"#)
        );
        let report = survey(&[&html]);
        assert_eq!(report.entries_scanned, 2);
        assert_eq!(report.multi_entry_records, 1);
        assert_eq!(report.clusters.len(), 2);
    }

    #[test]
    fn too_many_kinds_needs_a_human() {
        let mut body = String::new();
        for index in 0..26 {
            body.push_str(&format!(r#"<i class="k{index}"></i>"#));
        }
        let html = entry(&body);
        let report = survey(&[&html]);
        assert_eq!(report.kinds.len(), 26);
        assert_eq!(report.verdict.grade, Verdict::Manual);
    }

    #[test]
    fn optional_blocks_report_presence() {
        let with_extra = entry(r#"<p class="a"></p><p class="b"></p>"#);
        let without_extra = entry(r#"<p class="a"></p>"#);
        let report = survey(&[&with_extra, &without_extra]);
        let optional = report
            .stats
            .optional
            .iter()
            .find(|kind| kind.kind == "p.b")
            .expect("p.b is optional");
        assert!((optional.presence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn depth_counts_nested_elements() {
        let html = entry(r#"<div class="s"><span class="x"></span></div>"#);
        let report = survey(&[&html]);
        assert_eq!(report.stats.max_depth, 2);
        assert_eq!(report.stats.mean_depth, 2.0);
    }

    #[test]
    fn limit_stops_reading() {
        let mut surveyor = Surveyor::new(SurveyOptions::with_limit(Some(1))).expect("options");
        assert!(surveyor.push("a", ENTRY_A));
        assert!(!surveyor.push("b", ENTRY_A));
        assert_eq!(surveyor.records(), 1);
        assert_eq!(surveyor.entries_scanned(), 1);
    }

    #[test]
    fn sample_names_are_safe_and_unique() {
        assert_eq!(sample_file_name("a/b c"), "a_b_c.html");
        assert_eq!(sample_file_name("___"), "entry.html");

        let mut surveyor = Surveyor::new(SurveyOptions::default()).expect("options");
        surveyor.push("same", ENTRY_A);
        surveyor.push("same", ENTRY_A);
        let report = surveyor.finish();
        let samples = &report.clusters[0].samples;
        assert_eq!(samples.len(), 2);
        assert_ne!(samples[0].file, samples[1].file);
    }

    #[test]
    fn markdown_lists_kinds_and_verdict() {
        let report = survey(&[ENTRY_A]);
        let markdown = report.to_markdown("demo.mdx", 12);
        assert!(markdown.contains("## Slot kinds (2)"));
        assert!(markdown.contains("## Verdict: EASY"));
        assert!(markdown.contains("demo.mdx"));
    }
}
