//! P7: resolve a source resource reference to the form L1 stores.
//!
//! Resolution is scheme based and dictionary agnostic: the engine never asks
//! which dictionary a fragment came from. Two families exist:
//!
//! * `sound://<path>` and plain asset paths (`img/x.png`, `/media/x.jpg`)
//!   belong to the pack and become `odc-resource://<path>`,
//! * anything the engine does not own (`data:`, `http(s)://`, `mailto:`,
//!   protocol-relative `//host/x`) is kept verbatim. Keeping it verbatim is a
//!   deliberate non-decision: a fragment must never lose a reference because a
//!   profile author did not know about a scheme. The caller records the reason
//!   in the self-check notes so the gap stays visible.

/// One source reference, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The `odc-resource://` key, or `None` when the reference stays verbatim.
    pub key: Option<String>,
    /// Why the reference was kept verbatim (`None` when it resolved).
    pub reason: Option<String>,
}

impl Resolved {
    /// Whether the reference became a pack resource.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.key.is_some()
    }

    /// The URL to write into the document.
    #[must_use]
    pub fn url(&self, original: &str) -> String {
        self.key.clone().unwrap_or_else(|| String::from(original))
    }
}

/// Resolve one reference (spec 3.5).
#[must_use]
pub fn resolve(reference: &str) -> Resolved {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return kept("empty reference");
    }
    if let Some(rest) = trimmed.strip_prefix("sound://") {
        return keyed(rest);
    }
    if trimmed.starts_with("//") {
        // Protocol-relative: the host belongs to whatever serves the document.
        return kept("protocol-relative reference");
    }
    if let Some(scheme) = scheme(trimmed) {
        return kept(format!("unsupported scheme '{scheme}'"));
    }
    keyed(trimmed)
}

/// Build the key for a pack path.
fn keyed(path: &str) -> Resolved {
    let key = crate::mdd_import::normalize_resource_key(path);
    if key.is_empty() {
        return kept("empty path");
    }
    Resolved {
        key: Some(format!("odc-resource://{key}")),
        reason: None,
    }
}

fn kept(reason: impl Into<String>) -> Resolved {
    Resolved {
        key: None,
        reason: Some(reason.into()),
    }
}

/// URL scheme of a reference, when it has one.
///
/// A scheme is the `[a-z][a-z0-9+.-]*` run before the first `:`, and it must
/// come before any `/`, `?` or `#` — otherwise the colon belongs to a path
/// segment (or to a Windows drive letter of an asset path), not to a scheme.
fn scheme(reference: &str) -> Option<&str> {
    let colon = reference.find(':')?;
    let head = &reference[..colon];
    let mut chars = head.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')) {
        return None;
    }
    let boundary = reference.find(['/', '?', '#']).unwrap_or(reference.len());
    if colon > boundary {
        return None;
    }
    Some(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(reference: &str) -> Option<String> {
        resolve(reference).key
    }

    #[test]
    fn sound_references_become_pack_resources() {
        assert_eq!(
            key("sound://hwd/bre/9/lpd__ware.mp3").as_deref(),
            Some("odc-resource://hwd/bre/9/lpd__ware.mp3")
        );
    }

    #[test]
    fn asset_paths_are_normalised() {
        assert_eq!(
            key("img/apple.png").as_deref(),
            Some("odc-resource://img/apple.png")
        );
        assert_eq!(
            key("./img/apple.png").as_deref(),
            Some("odc-resource://img/apple.png")
        );
        assert_eq!(
            key("/media/english/cooki.jpg").as_deref(),
            Some("odc-resource://media/english/cooki.jpg")
        );
    }

    #[test]
    fn unknown_schemes_are_kept_verbatim_and_explained() {
        let resolved = resolve("https://example.com/x.png");
        assert!(!resolved.is_resolved());
        assert_eq!(
            resolved.url("https://example.com/x.png"),
            "https://example.com/x.png"
        );
        assert!(resolved.reason.unwrap_or_default().contains("scheme"));
        assert!(!resolve("data:image/png;base64,AAA").is_resolved());
        assert!(!resolve("//cdn.example.com/x.png").is_resolved());
    }

    #[test]
    fn empty_references_are_reported() {
        assert!(!resolve("   ").is_resolved());
        assert!(!resolve("sound://").is_resolved());
    }

    #[test]
    fn colons_inside_a_path_are_not_schemes() {
        assert_eq!(
            key("media/a:b/x.png").as_deref(),
            Some("odc-resource://media/a:b/x.png")
        );
    }
}
