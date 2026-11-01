import Foundation
import OSLog

/// Reader diagnostics; a damaged file is reported, never fatal.
private let readerLog = Logger(subsystem: "com.daxmate.odict", category: "reader")

/// One entry the reader remembers, as a bookmark or a history record.
///
/// The pair (`dictionaryID`, `key`) identifies an entry across launches: the
/// same headword in two dictionaries is two distinct references, while the same
/// entry opened twice is one.
public struct EntryRef: Codable, Identifiable, Sendable, Equatable {
    /// `LibraryItem.id` of the dictionary holding the entry.
    public let dictionaryID: String
    /// Normalised lookup key inside that dictionary.
    public let key: String
    /// Spelling shown to readers.
    public let headword: String
    /// When the entry was bookmarked or last opened.
    public let timestamp: Date

    /// Stable per dictionary, so a list can be keyed without collisions.
    public var id: String { "\(dictionaryID)#\(key)" }

    public init(
        dictionaryID: String,
        key: String,
        headword: String,
        timestamp: Date = Date()
    ) {
        self.dictionaryID = dictionaryID
        self.key = key
        self.headword = headword
        self.timestamp = timestamp
    }
}

/// Everything the reader keeps about entries, as stored in `reader.json`.
public struct ReaderState: Codable, Sendable, Equatable {
    /// Bookmarked entries, newest first.
    public var bookmarks: [EntryRef]
    /// Recently opened entries, newest first.
    public var history: [EntryRef]

    public init(bookmarks: [EntryRef] = [], history: [EntryRef] = []) {
        self.bookmarks = bookmarks
        self.history = history
    }
}

/// Outcome of reading `reader.json`, so the caller can decide whether the
/// reset is worth a notice.
public struct ReaderLoadResult: Sendable, Equatable {
    public let state: ReaderState
    /// True when a file existed but could not be decoded.
    public let recoveredFromCorruptFile: Bool
}

/// On-disk storage for bookmarks and query history.
///
/// Lives next to `library.json` and follows the same rules: JSON, written
/// atomically, and a file that cannot be read is replaced by an empty state
/// instead of stopping the reader.
public enum ReaderStore {
    public static let fileName = "reader.json"
    /// How many entries the history keeps before the oldest are dropped.
    public static let historyLimit = 200

    public static func fileURL(in directory: URL) -> URL {
        directory.appending(path: fileName, directoryHint: .notDirectory)
    }

    /// Load the reader state; a missing or undecodable file yields an empty one.
    public static func load(in directory: URL) -> ReaderState {
        loadResult(in: directory).state
    }

    /// Load the reader state and say whether a damaged file was discarded.
    public static func loadResult(in directory: URL) -> ReaderLoadResult {
        let url = fileURL(in: directory)
        guard let data = try? Data(contentsOf: url) else {
            return ReaderLoadResult(state: ReaderState(), recoveredFromCorruptFile: false)
        }
        guard let decoded = try? makeDecoder().decode(ReaderState.self, from: data) else {
            readerLog.warning("discarded unreadable reader state at \(url.path, privacy: .public)")
            return ReaderLoadResult(state: ReaderState(), recoveredFromCorruptFile: true)
        }
        return ReaderLoadResult(state: decoded.normalized(), recoveredFromCorruptFile: false)
    }

    /// Write the reader state atomically. Missing parent directories are created.
    public static func save(_ state: ReaderState, in directory: URL) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let data = try makeEncoder().encode(state.normalized())
        try data.write(to: fileURL(in: directory), options: [.atomic])
    }

    /// Remember an entry as the most recent one, without duplicating it.
    public static func recordHistory(_ ref: EntryRef, in state: inout ReaderState) {
        state.history.removeAll { $0.dictionaryID == ref.dictionaryID && $0.key == ref.key }
        state.history.insert(ref, at: 0)
        if state.history.count > historyLimit {
            state.history.removeLast(state.history.count - historyLimit)
        }
    }

    /// Add `ref` to the bookmarks, or drop it when it is already there.
    public static func toggleBookmark(_ ref: EntryRef, in state: inout ReaderState) {
        if let index = state.bookmarks.firstIndex(where: {
            $0.dictionaryID == ref.dictionaryID && $0.key == ref.key
        }) {
            state.bookmarks.remove(at: index)
        } else {
            state.bookmarks.append(ref)
        }
        state.bookmarks.sort { $0.timestamp > $1.timestamp }
    }

    /// Drop every reference to a dictionary that left the library.
    public static func forget(dictionaryID: String, in state: inout ReaderState) {
        state.bookmarks.removeAll { $0.dictionaryID == dictionaryID }
        state.history.removeAll { $0.dictionaryID == dictionaryID }
    }

    private static func makeEncoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }

    private static func makeDecoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}

extension ReaderState {
    /// Bookmark order and the history cap are invariants, not file contents.
    ///
    /// A hand-edited or older file may carry duplicates, unsorted bookmarks or
    /// a history longer than the limit; normalising on read keeps every other
    /// code path simple.
    func normalized() -> ReaderState {
        var bookmarks: [EntryRef] = []
        var seenBookmarks: Set<String> = []
        for ref in self.bookmarks.sorted(by: { $0.timestamp > $1.timestamp })
        where seenBookmarks.insert(ref.id).inserted {
            bookmarks.append(ref)
        }

        var history: [EntryRef] = []
        var seenHistory: Set<String> = []
        for ref in self.history where seenHistory.insert(ref.id).inserted {
            history.append(ref)
        }
        if history.count > ReaderStore.historyLimit {
            history.removeLast(history.count - ReaderStore.historyLimit)
        }

        return ReaderState(bookmarks: bookmarks, history: history)
    }
}
