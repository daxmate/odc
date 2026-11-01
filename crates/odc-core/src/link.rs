//! `@@@LINK=` redirects.
//!
//! MDict writes a cross-reference entry as a body that is exactly
//! `@@@LINK=<target>`; readers follow it instead of showing the line as
//! text. Dictionaries chain these (a plural form pointing at the lemma, the
//! lemma pointing at a synonym), so resolution walks the chain and refuses to
//! loop.

use crate::entry::{normalize_key, Entry};

/// Prefix that marks a redirect body.
pub const LINK_PREFIX: &str = "@@@LINK=";

/// Redirects followed before a lookup gives up.
pub const MAX_REDIRECT_HOPS: usize = 8;

/// Target of a redirect body, or `None` when `raw_html` is real markup.
#[must_use]
pub fn redirect_target(raw_html: &str) -> Option<&str> {
    let target = raw_html.trim().strip_prefix(LINK_PREFIX)?.trim();
    (!target.is_empty()).then_some(target)
}

/// Outcome of following `@@@LINK=` redirects from a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// The word, or the end of its redirect chain, is a real entry.
    Found {
        /// Key the caller asked for, normalised.
        requested: String,
        /// Key of the entry that was finally found.
        key: String,
        /// Redirects followed to reach it (`0` for a direct hit).
        hops: usize,
        /// The entry itself.
        entry: Entry,
    },
    /// A redirect pointed at a word the dictionary does not carry.
    Missing {
        /// Key the caller asked for, normalised.
        requested: String,
        /// Key the last redirect pointed at.
        target: String,
        /// Redirects followed before the miss.
        hops: usize,
    },
    /// The chain came back to a word it had already visited.
    Cycle {
        /// Key the caller asked for, normalised.
        requested: String,
        /// Keys visited, starting with the requested one.
        chain: Vec<String>,
    },
    /// The chain was longer than the hop limit.
    TooLong {
        /// Key the caller asked for, normalised.
        requested: String,
        /// Keys visited, starting with the requested one.
        chain: Vec<String>,
    },
    /// The word itself is absent.
    NotFound {
        /// Key the caller asked for, normalised.
        requested: String,
    },
}

impl LinkResolution {
    /// Key the caller asked for, normalised.
    #[must_use]
    pub fn requested(&self) -> &str {
        match self {
            Self::Found { requested, .. }
            | Self::Missing { requested, .. }
            | Self::Cycle { requested, .. }
            | Self::TooLong { requested, .. }
            | Self::NotFound { requested } => requested,
        }
    }

    /// Final entry key, when the lookup found one.
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Found { key, .. } => Some(key),
            _ => None,
        }
    }

    /// True when at least one redirect was followed to reach the entry.
    #[must_use]
    pub fn redirected(&self) -> bool {
        matches!(self, Self::Found { hops, .. } if *hops > 0)
    }
}

/// Follows `@@@LINK=` redirects starting at `word`.
///
/// `resolve` returns the entry stored under a normalised key; it is a
/// closure so the walk can be unit-tested without a repository.
pub fn resolve<F>(word: &str, mut resolve: F) -> crate::Result<LinkResolution>
where
    F: FnMut(&str) -> crate::Result<Option<Entry>>,
{
    let requested = normalize_key(word);
    let mut current = requested.clone();
    let mut chain = vec![current.clone()];
    let mut hops = 0usize;

    loop {
        let Some(entry) = resolve(&current)? else {
            return Ok(if hops == 0 {
                LinkResolution::NotFound { requested }
            } else {
                LinkResolution::Missing {
                    requested,
                    target: current,
                    hops,
                }
            });
        };
        let Some(target) = entry.raw_html.as_deref().and_then(redirect_target) else {
            return Ok(LinkResolution::Found {
                requested,
                key: current,
                hops,
                entry,
            });
        };
        let next = normalize_key(target);
        if chain.contains(&next) {
            return Ok(LinkResolution::Cycle { requested, chain });
        }
        if hops >= MAX_REDIRECT_HOPS {
            return Ok(LinkResolution::TooLong { requested, chain });
        }
        hops += 1;
        chain.push(next.clone());
        current = next;
    }
}
