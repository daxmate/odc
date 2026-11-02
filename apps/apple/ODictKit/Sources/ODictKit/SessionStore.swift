import Foundation
import OSLog

/// Session diagnostics; a damaged file is ignored rather than fatal.
private let sessionLog = Logger(subsystem: "com.daxmate.odict", category: "session")

/// Where the reader left off: the dictionary, the entry and the query.
///
/// Every field is optional so an old file that only carries some of them keeps
/// working. Empty strings are normalised to `nil` before the file is written.
public struct ReaderSession: Codable, Sendable, Equatable {
    /// `LibraryItem.id` of the dictionary that was active.
    public var dictionaryID: String?
    /// Lookup key of the entry that was on screen.
    public var key: String?
    /// Search text as it was submitted, without surrounding whitespace.
    public var query: String?

    public init(dictionaryID: String? = nil, key: String? = nil, query: String? = nil) {
        self.dictionaryID = dictionaryID
        self.key = key
        self.query = query
    }

    public static let empty = ReaderSession()

    /// True when there is nothing worth restoring.
    public var isEmpty: Bool {
        dictionaryID == nil && key == nil && query == nil
    }

    /// Trim every field and drop the ones that ended up blank.
    public func sanitized() -> ReaderSession {
        ReaderSession(
            dictionaryID: Self.clean(dictionaryID),
            key: Self.clean(key),
            query: Self.clean(query)
        )
    }

    private static func clean(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty
        else { return nil }
        return trimmed
    }
}

/// On-disk storage for the reader session.
///
/// Lives next to `library.json` and `reader.json` and follows the same rules:
/// JSON, written atomically, and a file that cannot be read is discarded
/// quietly instead of blocking the launch.
public enum SessionStore {
    /// File name of the persisted session inside the application support folder.
    public static let fileName = "session.json"

    public static func fileURL(in directory: URL) -> URL {
        directory.appending(path: fileName, directoryHint: .notDirectory)
    }

    /// Load the session; a missing or unreadable file yields an empty session.
    public static func load(in directory: URL) -> ReaderSession {
        loadResult(in: directory).session
    }

    /// Load the session and report whether a damaged file was discarded.
    public static func loadResult(in directory: URL) -> SessionLoadResult {
        let url = fileURL(in: directory)
        guard let data = try? Data(contentsOf: url) else {
            return SessionLoadResult(session: .empty, recoveredFromCorruptFile: false)
        }
        guard let decoded = try? decoder().decode(ReaderSession.self, from: data) else {
            sessionLog.warning("discarded unreadable session at \(url.path, privacy: .public)")
            return SessionLoadResult(session: .empty, recoveredFromCorruptFile: true)
        }
        return SessionLoadResult(session: decoded.sanitized(), recoveredFromCorruptFile: false)
    }

    /// Write the session atomically. Missing parent directories are created.
    ///
    /// An empty session removes the file so a later launch has nothing to
    /// restore and does not re-read stale identifiers.
    public static func save(_ session: ReaderSession, in directory: URL) throws {
        let cleaned = session.sanitized()
        let url = fileURL(in: directory)
        guard !cleaned.isEmpty else {
            try? FileManager.default.removeItem(at: url)
            return
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let data = try encoder().encode(cleaned)
        try data.write(to: url, options: [.atomic])
    }

    private static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }

    private static func decoder() -> JSONDecoder {
        JSONDecoder()
    }
}

/// Outcome of reading `session.json`.
public struct SessionLoadResult: Sendable, Equatable {
    public let session: ReaderSession
    /// True when a file existed but could not be decoded.
    public let recoveredFromCorruptFile: Bool
}

/// What the store should restore, after the identifiers were checked against
/// the dictionaries that are actually open.
struct SessionRestorePlan: Equatable, Sendable {
    let dictionaryID: UUID
    let key: String?
    let query: String?
}

/// Checks a stored session against live dictionaries.
@MainActor
protocol SessionLookup {
    func sessionHasDictionary(id: UUID) -> Bool
    func sessionHasEntry(key: String, in dictionaryID: UUID) -> Bool
}

/// Turns a stored session into a plan, dropping the parts that no longer exist.
enum SessionRestore {
    /// Returns `nil` when the dictionary is gone; a missing entry only drops
    /// the `key`, so the stored query still comes back.
    @MainActor
    static func plan(for session: ReaderSession, using lookup: SessionLookup) -> SessionRestorePlan? {
        let cleaned = session.sanitized()
        guard let idText = cleaned.dictionaryID,
              let dictionaryID = UUID(uuidString: idText),
              lookup.sessionHasDictionary(id: dictionaryID)
        else { return nil }

        let key: String? = if let candidate = cleaned.key,
                              lookup.sessionHasEntry(key: candidate, in: dictionaryID) {
            candidate
        } else {
            nil
        }
        return SessionRestorePlan(dictionaryID: dictionaryID, key: key, query: cleaned.query)
    }
}
