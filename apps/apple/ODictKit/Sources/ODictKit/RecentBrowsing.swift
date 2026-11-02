import Foundation

/// One entry in the "recently viewed" list shown while the search field is
/// empty.
public struct RecentBrowsingRow: Identifiable, Equatable, Sendable {
    public let ref: EntryRef
    public let dictionaryName: String

    public var id: String { ref.id }

    public init(ref: EntryRef, dictionaryName: String) {
        self.ref = ref
        self.dictionaryName = dictionaryName
    }
}

/// Builds the recent-browsing rows from the reader history.
public enum RecentBrowsing {
    /// The list never grows past this many rows.
    public static let limit = 8

    /// Newest first, de-duplicated, capped at `limit`.
    public static func rows(
        from history: [EntryRef],
        dictionaryNames: [UUID: String],
        limit: Int = RecentBrowsing.limit
    ) -> [RecentBrowsingRow] {
        guard limit > 0 else { return [] }
        var seen = Set<String>()
        var rows: [RecentBrowsingRow] = []
        for ref in history {
            guard seen.insert(ref.id).inserted else { continue }
            rows.append(
                RecentBrowsingRow(
                    ref: ref,
                    dictionaryName: name(for: ref, dictionaryNames: dictionaryNames)
                )
            )
            if rows.count == limit { break }
        }
        return rows
    }

    private static func name(for ref: EntryRef, dictionaryNames: [UUID: String]) -> String {
        guard let id = UUID(uuidString: ref.dictionaryID) else { return ref.dictionaryID }
        return dictionaryNames[id] ?? "已移除的词典"
    }
}
