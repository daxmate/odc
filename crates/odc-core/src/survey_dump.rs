//! Deterministic stratified sampling of dictionary body records.
//!
//! The odc survey --dump-jsonl flag uses this module to turn a whole
//! dictionary into a small sample for profile_compare --source. The rules are
//! deliberately boring so that the same dictionary always produces the same
//! sample:
//!
//! 1. body records are bucketed by the first character of their headword:
//!    A..Z for ASCII letters, # for digits, punctuation and anything else
//!    (non-ASCII letters included);
//! 2. every bucket receives a quota proportional to its share of all body
//!    records, never below SamplePlan::min_per_bucket while the bucket has
//!    that many records and the overall budget allows it;
//! 3. a bucket is filled by walking its records in file order and taking every
//!    stride-th one, where the stride is derived from the bucket size and its
//!    quota unless the caller pins it with SamplePlan::stride.
//!
//! Alias records (@@@LINK= pointers) and empty bodies are never sampled,
//! because the source contract compares fragments, not pointers.

use std::collections::BTreeMap;

use crate::survey::ALIAS_PREFIX;
use crate::Result;

/// Bucket label used for keys that do not start with an ASCII letter.
pub const OTHER_BUCKET: char = '#';

/// One body record that survived sampling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampledRecord {
    /// Headword exactly as stored in the dictionary.
    pub key: String,
    /// Raw record body, i.e. the HTML fragment a profile is asked to adapt.
    pub text: String,
}

/// Sampling knobs.
#[derive(Clone, Copy, Debug)]
pub struct SamplePlan {
    /// Maximum number of records to keep.
    pub limit: usize,
    /// Lower bound for a bucket quota, as long as the budget allows it.
    pub min_per_bucket: usize,
    /// Fixed stride inside every bucket (None derives it from size / quota).
    pub stride: Option<usize>,
}

impl Default for SamplePlan {
    fn default() -> Self {
        Self {
            limit: 500,
            min_per_bucket: 5,
            stride: None,
        }
    }
}

/// Per-bucket accounting, printed with the survey summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketSample {
    /// Bucket label (A..Z or #).
    pub bucket: char,
    /// Body records available in this bucket.
    pub available: usize,
    /// Records the plan asked for.
    pub quota: usize,
    /// Records actually taken.
    pub taken: usize,
    /// Distance between taken records inside this bucket.
    pub stride: usize,
}

/// Result of one sampling run.
#[derive(Clone, Debug)]
pub struct SampleReport {
    /// Body records seen (aliases and empty records excluded).
    pub body: usize,
    /// Records skipped because they only link to another headword.
    pub alias: usize,
    /// Records skipped because the body was empty.
    pub empty: usize,
    /// Per-bucket accounting, letters first and # last.
    pub buckets: Vec<BucketSample>,
    /// The sampled records, in dictionary file order.
    pub records: Vec<SampledRecord>,
}

/// Bucket of one headword: its first ASCII letter, or # for everything else.
pub fn bucket_label(key: &str) -> char {
    match key.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
        _ => OTHER_BUCKET,
    }
}

/// Order labels for reporting: A..Z, then #.
fn report_order(sizes: &BTreeMap<char, usize>) -> Vec<char> {
    let mut labels: Vec<char> = sizes.keys().copied().collect();
    labels.sort_by_key(|label| (*label == OTHER_BUCKET, *label));
    labels
}

/// Quotas for one set of bucket sizes (pure, deterministic).
fn quotas(sizes: &BTreeMap<char, usize>, plan: SamplePlan) -> BTreeMap<char, usize> {
    let mut quota = BTreeMap::new();
    let total: usize = sizes.values().sum();
    if total == 0 {
        return quota;
    }
    let target = plan.limit.min(total);
    for (&bucket, &size) in sizes {
        let share = target * size / total;
        let floor = plan.min_per_bucket.min(size);
        quota.insert(bucket, share.max(floor).min(size));
    }
    let mut assigned: usize = quota.values().sum();

    // Proportional rounding leaves records on the table: hand them to the
    // bucket with the most unused capacity, earliest label first on ties.
    while assigned < target {
        let mut best: Option<(char, usize)> = None;
        for (&bucket, &size) in sizes {
            let taken = quota.get(&bucket).copied().unwrap_or(0);
            if taken >= size {
                continue;
            }
            let room = size - taken;
            if best.is_none_or(|(_, best_room)| room > best_room) {
                best = Some((bucket, room));
            }
        }
        let Some((bucket, _)) = best else { break };
        if let Some(value) = quota.get_mut(&bucket) {
            *value += 1;
            assigned += 1;
        }
    }

    // The per-bucket floor can overshoot a tiny budget: trim the largest
    // quotas first, and only dip below the floor when nothing else is left.
    while assigned > target {
        let mut best: Option<(char, usize, bool)> = None;
        for (&bucket, &size) in sizes {
            let taken = quota.get(&bucket).copied().unwrap_or(0);
            if taken == 0 {
                continue;
            }
            let floor = plan.min_per_bucket.min(size);
            let above = taken > floor;
            let better = match best {
                None => true,
                Some((_, best_taken, best_above)) => {
                    (above && !best_above) || (above == best_above && taken > best_taken)
                }
            };
            if better {
                best = Some((bucket, taken, above));
            }
        }
        let Some((bucket, _, _)) = best else { break };
        if let Some(value) = quota.get_mut(&bucket) {
            *value -= 1;
            assigned -= 1;
        }
    }
    quota
}

/// Distance between taken records for one bucket.
fn stride_for(size: usize, quota: usize, pinned: Option<usize>) -> usize {
    if let Some(stride) = pinned {
        return stride.max(1);
    }
    if quota == 0 {
        return 1;
    }
    (size / quota).max(1)
}

/// Positions inside one bucket that the plan takes (ascending, deduplicated).
fn selection(size: usize, quota: usize, stride: usize) -> Vec<usize> {
    let mut picked = Vec::with_capacity(quota);
    let mut index = 0;
    while index < size && picked.len() < quota {
        picked.push(index);
        index += stride;
    }
    // A coarse stride can walk past the end early; fill the remaining quota in
    // file order so the bucket still contributes what was promised.
    let mut cursor = 0;
    while picked.len() < quota && cursor < size {
        if !picked.contains(&cursor) {
            picked.push(cursor);
        }
        cursor += 1;
    }
    picked.sort_unstable();
    picked
}

/// Sample body records from a re-openable record iterator.
///
/// The closure must build a fresh iterator every call; it is called twice, so
/// the dictionary is read twice and only the selected records are kept in
/// memory.
pub fn sample<I, F>(mut records: F, plan: SamplePlan) -> Result<SampleReport>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<SampledRecord>>,
{
    let mut sizes: BTreeMap<char, usize> = BTreeMap::new();
    let mut alias = 0usize;
    let mut empty = 0usize;
    for item in records() {
        let record = item?;
        if record.text.starts_with(ALIAS_PREFIX) {
            alias += 1;
            continue;
        }
        if record.text.trim().is_empty() {
            empty += 1;
            continue;
        }
        *sizes.entry(bucket_label(&record.key)).or_default() += 1;
    }
    let body: usize = sizes.values().sum();
    let quota = quotas(&sizes, plan);
    let order = report_order(&sizes);

    let mut strides: BTreeMap<char, usize> = BTreeMap::new();
    let mut picks: BTreeMap<char, Vec<usize>> = BTreeMap::new();
    for (&bucket, &size) in &sizes {
        let wanted = quota.get(&bucket).copied().unwrap_or(0);
        let stride = stride_for(size, wanted, plan.stride);
        strides.insert(bucket, stride);
        picks.insert(bucket, selection(size, wanted, stride));
    }

    let mut cursors: BTreeMap<char, usize> = BTreeMap::new();
    let mut taken: BTreeMap<char, usize> = BTreeMap::new();
    let mut seen: BTreeMap<char, usize> = BTreeMap::new();
    let mut sampled: Vec<SampledRecord> = Vec::with_capacity(plan.limit);
    for item in records() {
        let record = item?;
        if record.text.starts_with(ALIAS_PREFIX) || record.text.trim().is_empty() {
            continue;
        }
        let bucket = bucket_label(&record.key);
        let index = seen.entry(bucket).or_insert(0);
        let position = *index;
        *index += 1;
        let Some(list) = picks.get(&bucket) else {
            continue;
        };
        let cursor = cursors.entry(bucket).or_insert(0);
        if *cursor < list.len() && list[*cursor] == position {
            *cursor += 1;
            *taken.entry(bucket).or_insert(0) += 1;
            sampled.push(record);
        }
    }

    let mut buckets = Vec::with_capacity(order.len());
    for bucket in order {
        let available = sizes.get(&bucket).copied().unwrap_or(0);
        let wanted = quota.get(&bucket).copied().unwrap_or(0);
        buckets.push(BucketSample {
            bucket,
            available,
            quota: wanted,
            taken: taken.get(&bucket).copied().unwrap_or(0),
            stride: strides.get(&bucket).copied().unwrap_or(1),
        });
    }
    Ok(SampleReport {
        body,
        alias,
        empty,
        buckets,
        records: sampled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(counts: &[(char, usize)]) -> BTreeMap<char, usize> {
        counts.iter().copied().collect()
    }

    #[test]
    fn buckets_use_the_first_ascii_letter() {
        assert_eq!(bucket_label("alpha"), 'A');
        assert_eq!(bucket_label("Zebra"), 'Z');
        assert_eq!(bucket_label("10,000-foot view"), OTHER_BUCKET);
        assert_eq!(bucket_label("-ware"), OTHER_BUCKET);
        assert_eq!(bucket_label("\u{e9}lan"), OTHER_BUCKET);
        assert_eq!(bucket_label(""), OTHER_BUCKET);
    }

    #[test]
    fn quotas_are_proportional_and_sum_to_the_budget() {
        let sizes = collect(&[('A', 6000), ('B', 3000), ('C', 1000)]);
        let plan = SamplePlan {
            limit: 500,
            ..SamplePlan::default()
        };
        let quota = quotas(&sizes, plan);
        assert_eq!(quota.values().sum::<usize>(), 500);
        assert_eq!(quota[&'A'], 300);
        assert_eq!(quota[&'B'], 150);
        assert_eq!(quota[&'C'], 50);
    }

    #[test]
    fn small_buckets_get_the_floor() {
        let sizes = collect(&[('A', 900), ('Q', 20), ('#', 80)]);
        let quota = quotas(
            &sizes,
            SamplePlan {
                limit: 100,
                ..SamplePlan::default()
            },
        );
        assert_eq!(quota.values().sum::<usize>(), 100);
        assert!(quota[&'Q'] >= 5);
        assert!(quota[&'#'] >= 5);
    }

    #[test]
    fn quotas_never_exceed_a_bucket() {
        let sizes = collect(&[('A', 2), ('B', 998)]);
        let quota = quotas(
            &sizes,
            SamplePlan {
                limit: 500,
                ..SamplePlan::default()
            },
        );
        assert_eq!(quota[&'A'], 2);
        assert_eq!(quota.values().sum::<usize>(), 500);
    }

    #[test]
    fn the_floor_yields_to_a_tiny_budget() {
        let sizes = collect(&[('A', 100), ('B', 100), ('C', 100)]);
        let quota = quotas(
            &sizes,
            SamplePlan {
                limit: 6,
                min_per_bucket: 5,
                stride: None,
            },
        );
        assert_eq!(quota.values().sum::<usize>(), 6);
    }

    #[test]
    fn selection_is_evenly_spaced_and_sorted() {
        let picked = selection(1000, 5, 200);
        assert_eq!(picked, vec![0, 200, 400, 600, 800]);
    }

    #[test]
    fn selection_fills_when_stride_overshoots() {
        let picked = selection(7, 4, 3);
        assert_eq!(picked.len(), 4);
        assert_eq!(picked, vec![0, 1, 3, 6]);
    }

    #[test]
    fn sample_skips_aliases_and_empty_bodies() {
        let rows: Vec<Result<SampledRecord>> = vec![
            Ok(SampledRecord {
                key: "able".into(),
                text: "<p>able</p>".into(),
            }),
            Ok(SampledRecord {
                key: "able-1".into(),
                text: "@@@LINK=able".into(),
            }),
            Ok(SampledRecord {
                key: "able-2".into(),
                text: "   ".into(),
            }),
            Ok(SampledRecord {
                key: "baker".into(),
                text: "<p>baker</p>".into(),
            }),
        ];
        let report = sample(
            || rows.clone().into_iter(),
            SamplePlan {
                limit: 10,
                ..SamplePlan::default()
            },
        )
        .expect("sample");
        assert_eq!(report.body, 2);
        assert_eq!(report.alias, 1);
        assert_eq!(report.empty, 1);
        assert_eq!(report.records.len(), 2);
        assert_eq!(report.records[0].key, "able");
        assert_eq!(report.records[1].key, "baker");
    }

    #[test]
    fn sample_is_reproducible() {
        let rows: Vec<Result<SampledRecord>> = (0..300)
            .map(|index| {
                Ok(SampledRecord {
                    key: format!("{}{index:03}", if index % 3 == 0 { "a" } else { "b" }),
                    text: format!("<p>{index}</p>"),
                })
            })
            .collect();
        let plan = SamplePlan {
            limit: 40,
            ..SamplePlan::default()
        };
        let first = sample(|| rows.clone().into_iter(), plan).expect("first");
        let second = sample(|| rows.clone().into_iter(), plan).expect("second");
        assert_eq!(first.records, second.records);
        assert_eq!(first.records.len(), 40);
    }
}
