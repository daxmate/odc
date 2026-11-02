import Foundation
import Observation

/// Recent search queries, newest first, persisted in user defaults.
@MainActor
@Observable
public final class SearchHistoryStore {
    /// Recorded queries, newest first.
    public private(set) var items: [String]

    @ObservationIgnored private let userDefaults: UserDefaults
    @ObservationIgnored private let storageKey: String
    @ObservationIgnored private let limit: Int

    public init(
        userDefaults: UserDefaults = .standard,
        storageKey: String = "ODict.searchHistory",
        limit: Int = 10
    ) {
        self.userDefaults = userDefaults
        self.storageKey = storageKey
        self.limit = limit
        self.items = Self.sanitize(userDefaults.stringArray(forKey: storageKey) ?? [], limit: limit)
    }

    /// Record `text` as the most recent query, moving an existing match to the front.
    public func record(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        var updated = items.filter { $0.caseInsensitiveCompare(trimmed) != .orderedSame }
        updated.insert(trimmed, at: 0)
        apply(updated)
    }

    /// Remove every entry matching `text` (case-insensitive).
    public func remove(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        apply(items.filter { $0.caseInsensitiveCompare(trimmed) != .orderedSame })
    }

    /// Drop the whole history.
    public func clear() {
        apply([])
    }

    private func apply(_ newItems: [String]) {
        let cleaned = Self.sanitize(newItems, limit: limit)
        items = cleaned
        userDefaults.set(cleaned, forKey: storageKey)
    }

    private static func sanitize(_ values: [String], limit: Int) -> [String] {
        var result: [String] = []
        for value in values {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }
            let duplicate = result.contains { $0.caseInsensitiveCompare(trimmed) == .orderedSame }
            guard !duplicate else { continue }
            result.append(trimmed)
            if result.count >= limit { break }
        }
        return result
    }
}
