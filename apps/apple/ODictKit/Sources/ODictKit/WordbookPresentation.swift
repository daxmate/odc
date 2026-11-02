import Foundation

/// How the word book orders its rows.
public enum WordbookSort: String, CaseIterable, Identifiable, Sendable {
    /// Newest first, the order `reader.json` already uses.
    case recent
    /// `localizedStandardCompare` on the headword.
    case alphabetical

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .recent: "最近"
        case .alphabetical: "字母"
        }
    }
}

/// Which records the word book shows.
public enum WordbookFilter: Hashable, Identifiable, Sendable {
    case all
    case bookmarks
    case history

    public var id: String { storageValue }

    /// Value kept in `@AppStorage`; the key prefix is `odict.wordbook.`.
    public var storageValue: String {
        switch self {
        case .all: "all"
        case .bookmarks: "bookmarks"
        case .history: "history"
        }
    }

    public var title: String {
        switch self {
        case .all: "全部"
        case .bookmarks: "收藏"
        case .history: "浏览历史"
        }
    }

    public init?(storageValue: String) {
        switch storageValue {
        case "all": self = .all
        case "bookmarks": self = .bookmarks
        case "history": self = .history
        default: return nil
        }
    }
}

/// One row of the word book, independent of how it is rendered.
public struct WordbookEntry: Identifiable, Equatable, Sendable {
    public let ref: EntryRef
    public let dictionaryName: String
    public let isBookmarked: Bool

    public var id: String { ref.id }

    public init(ref: EntryRef, dictionaryName: String, isBookmarked: Bool) {
        self.ref = ref
        self.dictionaryName = dictionaryName
        self.isBookmarked = isBookmarked
    }
}

/// The filtered and sorted rows of the word book.
///
/// Kept out of the view so the combinations can be tested without SwiftUI.
public struct WordbookPresentation: Equatable, Sendable {
    public let entries: [WordbookEntry]
    public let emptyMessage: String

    public var isEmpty: Bool { entries.isEmpty }
    public var count: Int { entries.count }

    /// - Parameters:
    ///   - bookmarks: `reader.json` bookmarks, newest first.
    ///   - history: `reader.json` history, newest first.
    ///   - filter: which records to show.
    ///   - dictionaryID: `nil` shows every dictionary.
    ///   - sort: row order.
    ///   - dictionaryNames: display name by `LibraryItem.id`.
    public init(
        bookmarks: [EntryRef],
        history: [EntryRef],
        filter: WordbookFilter,
        dictionaryID: UUID?,
        sort: WordbookSort,
        dictionaryNames: [UUID: String]
    ) {
        let bookmarkIDs = Set(bookmarks.map(\.id))
        let historyIDs = Set(history.map(\.id))

        // Union of both lists, keyed by entry id. The later timestamp wins so
        // an entry that was opened again moves to the front of "recent".
        var merged: [String: (ref: EntryRef, isBookmarked: Bool)] = [:]
        for ref in history {
            merged[ref.id] = (ref, bookmarkIDs.contains(ref.id))
        }
        for ref in bookmarks {
            if let existing = merged[ref.id], existing.ref.timestamp >= ref.timestamp {
                merged[ref.id] = (existing.ref, true)
            } else {
                merged[ref.id] = (ref, true)
            }
        }

        let filtered = merged.values.filter { candidate in
            if let dictionaryID, candidate.ref.dictionaryID != dictionaryID.uuidString {
                return false
            }
            switch filter {
            case .all:
                return true
            case .bookmarks:
                return candidate.isBookmarked
            case .history:
                return historyIDs.contains(candidate.ref.id)
            }
        }

        let rows = filtered.map { candidate in
            WordbookEntry(
                ref: candidate.ref,
                dictionaryName: Self.name(
                    for: candidate.ref,
                    dictionaryNames: dictionaryNames
                ),
                isBookmarked: candidate.isBookmarked
            )
        }

        let ordered: [WordbookEntry]
        switch sort {
        case .recent:
            ordered = rows.sorted { lhs, rhs in
                if lhs.ref.timestamp != rhs.ref.timestamp {
                    return lhs.ref.timestamp > rhs.ref.timestamp
                }
                return Self.compare(lhs.ref.headword, rhs.ref.headword)
            }
        case .alphabetical:
            ordered = rows.sorted { lhs, rhs in
                let result = lhs.ref.headword.localizedStandardCompare(rhs.ref.headword)
                if result != .orderedSame { return result == .orderedAscending }
                return Self.compare(lhs.ref.headword, rhs.ref.headword)
            }
        }

        entries = ordered
        emptyMessage = Self.emptyMessage(filter: filter, dictionaryID: dictionaryID)
    }

    private static func compare(_ lhs: String, _ rhs: String) -> Bool {
        lhs.localizedStandardCompare(rhs) == .orderedAscending
    }

    private static func name(for ref: EntryRef, dictionaryNames: [UUID: String]) -> String {
        guard let id = UUID(uuidString: ref.dictionaryID) else { return ref.dictionaryID }
        return dictionaryNames[id] ?? "已移除的词典"
    }

    private static func emptyMessage(filter: WordbookFilter, dictionaryID: UUID?) -> String {
        if dictionaryID != nil {
            return "这本词典还没有记录"
        }
        switch filter {
        case .all: return "生词本还是空的"
        case .bookmarks: return "还没有收藏"
        case .history: return "还没有浏览记录"
        }
    }
}
