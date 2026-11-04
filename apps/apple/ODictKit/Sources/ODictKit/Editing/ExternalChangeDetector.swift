import Foundation

/// What re-reading the repository HEAD says about work done elsewhere.
public enum ExternalChangeOutcome: Equatable, Sendable {
    /// HEAD is the one this session started from.
    case inSync
    /// HEAD moved and there was nothing of ours to lose, so the caller may
    /// reload silently.
    case reloaded
    /// HEAD moved while the working set still holds uncommitted edits. The
    /// caller has to ask before overwriting anything.
    case conflict
}

/// Tracks the HEAD a session was loaded from and classifies later reads.
///
/// The detector owns no I/O: callers read `WorkingStatus.head` themselves and
/// hand it in, which keeps the state machine testable without a repository.
public struct ExternalChangeDetector: Equatable, Sendable {
    /// HEAD the session last agreed with.
    public private(set) var baselineHead: String?

    public init(baselineHead: String? = nil) {
        self.baselineHead = baselineHead
    }

    /// Re-baselines without reporting a change (first load, our own commit,
    /// or an explicit reload the user asked for).
    public mutating func adopt(head: String?) {
        baselineHead = head
    }

    /// Compares a freshly read HEAD against the baseline.
    ///
    /// A clean working set is adopted immediately so the same foreign commit
    /// is only announced once.
    public mutating func evaluate(head: String?, hasLocalChanges: Bool) -> ExternalChangeOutcome {
        guard head != baselineHead else { return .inSync }
        if hasLocalChanges { return .conflict }
        baselineHead = head
        return .reloaded
    }
}
