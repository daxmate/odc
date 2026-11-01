import CryptoKit
import Foundation

/// One dictionary the app has copied into its own storage.
///
/// `library.json` holds one of these per import so a relaunch can reopen the
/// copy instead of asking the reader to pick the file again.
public struct LibraryItem: Codable, Identifiable, Sendable, Equatable {
    /// Identifier of the copy: the `<uuid>.odc` directory name without extension.
    ///
    /// Mutable because migrating a copy into the library folder can rename it
    /// when its old name is taken; `storedName` and `id` move together.
    public var id: UUID
    /// Name shown in the UI.
    public var displayName: String
    /// Directory holding `odc.db`, relative to the library directory.
    public var storedName: String
    /// Name of the file (or directory) the reader picked.
    public var sourceName: String
    /// Size of the picked source, in bytes.
    public var sourceBytes: Int64
    /// SHA-256 over the first `LibraryFileFingerprint.prefixBytes` of the source.
    public var sourceHash: String?
    /// When the copy was registered.
    public var importedAt: Date

    public init(
        id: UUID = UUID(),
        displayName: String,
        storedName: String? = nil,
        sourceName: String,
        sourceBytes: Int64,
        sourceHash: String? = nil,
        importedAt: Date = Date()
    ) {
        self.id = id
        self.displayName = displayName
        self.storedName = storedName ?? "\(id.uuidString).odc"
        self.sourceName = sourceName
        self.sourceBytes = sourceBytes
        self.sourceHash = sourceHash
        self.importedAt = importedAt
    }
}

extension LibraryItem {
    /// True for a copy the app found on disk without knowing its source file.
    ///
    /// Placeholders carry a generated name and no source identity; a later
    /// import of the same file replaces the descriptive fields in place.
    public var isPlaceholder: Bool {
        sourceName.isEmpty
    }

    /// Identity a fresh import of the same file would produce.
    public var sourceDescriptor: LibrarySourceDescriptor {
        LibrarySourceDescriptor(name: sourceName, bytes: sourceBytes, contentHash: sourceHash)
    }

    /// True when `source` is the same picked file as this item.
    ///
    /// A stored hash that disagrees with the incoming one wins: same name and
    /// size are not enough when both sides know their content.
    public func matches(_ source: LibrarySourceDescriptor) -> Bool {
        // Size first: two files of different sizes are different dictionaries,
        // however much of their head they share.
        guard sourceBytes == source.bytes else { return false }
        if let hash = source.contentHash, let stored = sourceHash {
            return hash == stored
        }
        return sourceName == source.name
    }

    public func matches(_ other: LibraryItem) -> Bool {
        matches(other.sourceDescriptor)
    }
}

/// What the reader picked, reduced to a value that survives a relaunch.
public struct LibrarySourceDescriptor: Equatable, Sendable {
    public let name: String
    public let bytes: Int64
    public let contentHash: String?

    public init(name: String, bytes: Int64, contentHash: String? = nil) {
        self.name = name
        self.bytes = bytes
        self.contentHash = contentHash
    }
}

/// Persisted library contents.
public struct LibraryState: Codable, Sendable, Equatable {
    /// Bumped when the on-disk shape changes; unknown versions are quarantined.
    public static let currentVersion = 1

    public var version: Int
    public var items: [LibraryItem]
    /// The dictionary the detail column followed; nil when nothing was open.
    public var lastOpenedID: UUID?
    /// Dictionaries left open at the last save, oldest first.
    ///
    /// Files written before multi-dictionary support carry no such key; they
    /// decode to an empty list and fall back to `lastOpenedID` on restore.
    public var openItemIDs: [UUID]

    public init(
        items: [LibraryItem] = [],
        lastOpenedID: UUID? = nil,
        openItemIDs: [UUID] = [],
        version: Int = LibraryState.currentVersion
    ) {
        self.version = version
        self.items = items
        self.lastOpenedID = lastOpenedID
        self.openItemIDs = openItemIDs
    }

    private enum CodingKeys: String, CodingKey {
        case version
        case items
        case lastOpenedID
        case openItemIDs
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        version = try container.decode(Int.self, forKey: .version)
        items = try container.decode([LibraryItem].self, forKey: .items)
        lastOpenedID = try container.decodeIfPresent(UUID.self, forKey: .lastOpenedID)
        openItemIDs = try container.decodeIfPresent([UUID].self, forKey: .openItemIDs) ?? []
    }

    public var isEmpty: Bool { items.isEmpty }

    public func item(id: UUID) -> LibraryItem? {
        items.first { $0.id == id }
    }
}

/// Outcome of `LibraryStore.deduplicateExistingCopies`.
public struct LibraryDeduplicationReport: Equatable, Sendable {
    /// Copy directories moved out of the library.
    public let removedStoredNames: [String]
    /// Bytes those copies occupied (they live on in `trashDirectory`).
    public let movedBytes: Int64
    /// Where the copies were moved, so they can be recovered by hand.
    public let trashDirectory: URL?

    public init(removedStoredNames: [String] = [], movedBytes: Int64 = 0, trashDirectory: URL? = nil) {
        self.removedStoredNames = removedStoredNames
        self.movedBytes = movedBytes
        self.trashDirectory = trashDirectory
    }

    public var removedCount: Int { removedStoredNames.count }
    public var isEmpty: Bool { removedStoredNames.isEmpty }
}

/// Reads and writes `library.json` next to the imported copies.
public enum LibraryStore {
    /// File name of the persisted library inside the application support folder.
    public static let fileName = "library.json"

    public static func fileURL(in directory: URL) -> URL {
        directory.appending(path: fileName, directoryHint: .notDirectory)
    }

    /// Load the library. A missing, unreadable or unknown-version file yields an
    /// empty state; a file that exists but cannot be decoded is moved aside
    /// (`.bak-<timestamp>`) so it is never silently overwritten.
    public static func load(from directory: URL) -> LibraryState {
        let url = fileURL(in: directory)
        guard let data = try? Data(contentsOf: url) else {
            return LibraryState()
        }
        let decoded = try? makeDecoder().decode(LibraryState.self, from: data)
        guard let state = decoded, state.version == LibraryState.currentVersion else {
            quarantine(url)
            return LibraryState()
        }
        return state
    }

    /// Write the library atomically. Missing parent directories are created.
    public static func save(_ state: LibraryState, to directory: URL) throws {
        let fileManager = FileManager.default
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        var versioned = state
        versioned.version = LibraryState.currentVersion
        let data = try makeEncoder().encode(versioned)
        try data.write(to: fileURL(in: directory), options: .atomic)
    }

    /// Register `item`, replacing any entry that describes the same source.
    ///
    /// When an entry already covers the source the existing entry (and therefore
    /// its copy on disk) is kept; only the descriptive fields are refreshed.
    public static func add(_ item: LibraryItem, to state: inout LibraryState) {
        let match = state.items.firstIndex {
            $0.id == item.id
                || (!item.isPlaceholder && !$0.isPlaceholder && $0.matches(item))
        }
        guard let index = match else {
            state.items.append(item)
            return
        }
        var existing = state.items[index]
        existing.displayName = item.displayName
        existing.sourceName = item.sourceName
        existing.sourceBytes = item.sourceBytes
        existing.sourceHash = item.sourceHash
        state.items[index] = existing
    }

    public static func remove(id: UUID, from state: inout LibraryState) {
        state.items.removeAll { $0.id == id }
        state.openItemIDs.removeAll { $0 == id }
        if state.lastOpenedID == id {
            state.lastOpenedID = nil
        }
    }

    /// Record `id` as open, keeping the open order.
    ///
    /// The oldest entries are dropped once `limit` dictionaries are open, so a
    /// stale file can never push the sidebar past its cap.
    public static func markOpen(_ id: UUID, in state: inout LibraryState, limit: Int) {
        state.openItemIDs.removeAll { $0 == id }
        state.openItemIDs.append(id)
        let excess = state.openItemIDs.count - max(limit, 0)
        if excess > 0 {
            state.openItemIDs.removeFirst(excess)
        }
    }

    /// The ids to reopen on launch, oldest first.
    ///
    /// Cleans `state` in place: a file written before multi-dictionary support
    /// falls back to `lastOpenedID`, ids whose library entry is gone are
    /// dropped, duplicates collapse, and only the newest `limit` survive.
    @discardableResult
    public static func restoreOpenIDs(in state: inout LibraryState, limit: Int) -> [UUID] {
        var ids = state.openItemIDs
        if ids.isEmpty,
           let lastOpenedID = state.lastOpenedID,
           state.item(id: lastOpenedID) != nil {
            ids = [lastOpenedID]
        }
        var seen: Set<UUID> = []
        ids = ids.filter { state.item(id: $0) != nil && seen.insert($0).inserted }
        if ids.count > max(limit, 0) {
            ids = Array(ids.suffix(max(limit, 0)))
        }
        state.openItemIDs = ids
        return ids
    }

    /// The entry for `source`, when the app already copied that exact file.
    public static func item(
        matching source: LibrarySourceDescriptor,
        in state: LibraryState
    ) -> LibraryItem? {
        state.items.first { $0.matches(source) }
    }

    /// Find a copy that predates `library.json` and holds exactly `source`.
    ///
    /// A standalone `.odc` file *is* an `odc.db`, so size plus head hash
    /// recognise it; copies the library already knows are skipped. This keeps a
    /// re-import of a dictionary copied by an older build from making a second
    /// 200 MB duplicate.
    public static func orphanCopy(
        in directory: URL,
        matching source: LibrarySourceDescriptor,
        state: LibraryState,
        fileManager: FileManager = .default
    ) throws -> LibraryItem? {
        guard let hash = source.contentHash else { return nil }
        let entries = try fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )
        for entry in entries.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            let values = try? entry.resourceValues(forKeys: [.isDirectoryKey, .creationDateKey])
            guard values?.isDirectory == true else { continue }
            let storedName = entry.lastPathComponent
            // Entries with a real source are already accounted for; a
            // placeholder is upgraded in place instead of copied again.
            let registered = state.items.first { $0.storedName == storedName }
            if let registered, !registered.isPlaceholder {
                continue
            }
            let database = entry.appending(path: "odc.db", directoryHint: .notDirectory)
            guard fileManager.fileExists(atPath: database.path) else { continue }
            let descriptor = try LibraryFileFingerprint.descriptor(for: database)
            guard descriptor.bytes == source.bytes, descriptor.contentHash == hash else {
                continue
            }
            return LibraryItem(
                id: registered?.id
                    ?? UUID(uuidString: entry.deletingPathExtension().lastPathComponent)
                    ?? UUID(),
                displayName: source.name,
                storedName: storedName,
                sourceName: source.name,
                sourceBytes: source.bytes,
                sourceHash: hash,
                importedAt: registered?.importedAt ?? Date()
            )
        }
        return nil
    }

    /// Copies on disk that `library.json` does not know about yet.
    ///
    /// Used on launch so a build that predates the library still shows the
    /// dictionaries the reader imported earlier. The source file is unknown, so
    /// the name is a placeholder (`未命名词典 <uuid 前 8 位>`) that a later
    /// import of the same file replaces; nothing is guessed from the content.
    public static func adoptableCopies(
        in directory: URL,
        registered: LibraryState = LibraryState(),
        fileManager: FileManager = .default
    ) -> [LibraryItem] {
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey, .creationDateKey],
            options: [.skipsHiddenFiles]
        ) else {
            return []
        }

        var items: [LibraryItem] = []
        for entry in entries.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            let storedName = entry.lastPathComponent
            if registered.items.contains(where: { $0.storedName == storedName }) {
                continue
            }
            let values = try? entry.resourceValues(forKeys: [.isDirectoryKey, .creationDateKey])
            guard values?.isDirectory == true else { continue }
            let database = entry.appending(path: "odc.db", directoryHint: .notDirectory)
            let databaseValues = try? database.resourceValues(forKeys: [.fileSizeKey])
            guard let size = databaseValues?.fileSize, size > 0,
                  fileManager.fileExists(atPath: database.path) else { continue }
            let id = UUID(uuidString: entry.deletingPathExtension().lastPathComponent) ?? UUID()
            items.append(
                LibraryItem(
                    id: id,
                    displayName: "未命名词典 \(id.uuidString.prefix(8))",
                    storedName: storedName,
                    sourceName: "",
                    sourceBytes: Int64(size),
                    sourceHash: nil,
                    importedAt: values?.creationDate ?? Date()
                )
            )
        }
        return items
    }

    /// The entry for the copy stored under `storedName`.
    public static func item(forStoredName storedName: String, in state: LibraryState) -> LibraryItem? {
        state.items.first { $0.storedName == storedName }
    }

    /// Directory of the copy, e.g. `<library>/<uuid>.odc`.
    ///
    /// Entries written by an older build can carry an absolute path instead of
    /// a name inside the library; resolving those keeps them openable so the
    /// startup migration can copy them in rather than drop them.
    public static func copyDirectory(for item: LibraryItem, in root: URL) -> URL {
        if item.storedName.hasPrefix("/") {
            return URL(fileURLWithPath: item.storedName, isDirectory: true)
        }
        return root.appending(path: item.storedName, directoryHint: .isDirectory)
    }

    /// Database the C ABI opens, e.g. `<library>/<uuid>.odc/odc.db`.
    public static func openTarget(for item: LibraryItem, in root: URL) -> URL {
        copyDirectory(for: item, in: root).appending(path: "odc.db", directoryHint: .notDirectory)
    }

    /// Drop entries whose copy disappeared (deleted by hand, interrupted write…).
    @discardableResult
    public static func pruneMissing(
        in state: inout LibraryState,
        directory: URL,
        fileManager: FileManager = .default
    ) -> [UUID] {
        let missing = state.items.filter {
            !fileManager.fileExists(atPath: openTarget(for: $0, in: directory).path)
        }
        guard !missing.isEmpty else { return [] }
        for item in missing {
            remove(id: item.id, from: &state)
        }
        return missing.map(\.id)
    }

    /// Move duplicate copies out of the library directory.
    ///
    /// Copies are grouped by `odc.db` size plus the hash of its first 16 MiB —
    /// the same key used for imports, so this stays cheap on 200 MB files. One
    /// copy per group is kept (the one the library points at, preferring
    /// `lastOpenedID`); the rest are moved to `trashDirectory` instead of being
    /// deleted, and their entries are dropped from `state`.
    ///
    /// This never runs on its own: the caller decides when to reclaim space.
    @discardableResult
    public static func deduplicateExistingCopies(
        in directory: URL,
        state: inout LibraryState,
        trashDirectory: URL? = nil,
        timestamp: Date = Date(),
        fileManager: FileManager = .default
    ) throws -> LibraryDeduplicationReport {
        let entries = try fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )
        let copies = entries
            .filter { $0.pathExtension == "odc" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }

        var groups: [String: [URL]] = [:]
        var sizes: [String: Int64] = [:]
        for copy in copies {
            let database = copy.appending(path: "odc.db", directoryHint: .notDirectory)
            guard fileManager.fileExists(atPath: database.path) else { continue }
            let descriptor = try LibraryFileFingerprint.descriptor(for: database)
            let key = "\(descriptor.bytes)|\(descriptor.contentHash ?? "-")"
            groups[key, default: []].append(copy)
            sizes[key] = descriptor.bytes
        }

        var removed: [String] = []
        var movedBytes: Int64 = 0
        var trash: URL?
        for key in groups.keys.sorted() {
            guard let group = groups[key], group.count > 1 else { continue }
            guard let keeper = keeper(in: group, state: state) else { continue }
            for copy in group where copy != keeper {
                let destination = try trashURL(
                    for: copy,
                    in: directory,
                    trashDirectory: trashDirectory,
                    timestamp: timestamp,
                    fileManager: fileManager
                )
                try fileManager.moveItem(at: copy, to: destination)
                trash = destination.deletingLastPathComponent()
                removed.append(copy.lastPathComponent)
                movedBytes += sizes[key] ?? 0
            }
        }

        guard !removed.isEmpty else {
            return LibraryDeduplicationReport()
        }
        for storedName in removed {
            if let item = item(forStoredName: storedName, in: state) {
                remove(id: item.id, from: &state)
            }
        }
        return LibraryDeduplicationReport(
            removedStoredNames: removed,
            movedBytes: movedBytes,
            trashDirectory: trash
        )
    }

    /// Copy kept for a group of identical dictionaries.
    private static func keeper(in copies: [URL], state: LibraryState) -> URL? {
        if let lastOpened = state.lastOpenedID,
           let item = state.item(id: lastOpened),
           let match = copies.first(where: { $0.lastPathComponent == item.storedName }) {
            return match
        }
        return copies.first { copy in
            state.items.contains { $0.storedName == copy.lastPathComponent }
        } ?? copies.first
    }

    private static func trashURL(
        for copy: URL,
        in directory: URL,
        trashDirectory: URL?,
        timestamp: Date,
        fileManager: FileManager
    ) throws -> URL {
        let root = trashDirectory ?? defaultTrashDirectory(in: directory, timestamp: timestamp)
        try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
        var destination = root.appending(path: copy.lastPathComponent, directoryHint: .isDirectory)
        var counter = 1
        while fileManager.fileExists(atPath: destination.path) {
            destination = root.appending(
                path: "\(copy.deletingPathExtension().lastPathComponent)-\(counter).odc",
                directoryHint: .isDirectory
            )
            counter += 1
        }
        return destination
    }

    /// `.Trash/<timestamp>` inside the library directory: recoverable, hidden
    /// from the finder, and outside the copies the app reopens.
    public static func defaultTrashDirectory(in directory: URL, timestamp: Date = Date()) -> URL {
        let stamp = ISO8601DateFormatter().string(from: timestamp)
            .replacingOccurrences(of: ":", with: "-")
        return directory
            .appending(path: ".Trash", directoryHint: .isDirectory)
            .appending(path: stamp, directoryHint: .isDirectory)
    }

    private static func quarantine(_ url: URL) {
        let stamp = ISO8601DateFormatter().string(from: Date())
            .replacingOccurrences(of: ":", with: "-")
        let backup = url.appendingPathExtension("bak-\(stamp)")
        try? FileManager.default.moveItem(at: url, to: backup)
    }

    // MARK: - Moving entries into the library folder

    /// One entry the startup migration could not move into the library folder.
    ///
    /// The entry stays in `library.json` exactly as it was, so a retry after
    /// the file becomes readable works and nothing is lost silently.
    public struct MigrationIssue: Equatable, Sendable, Identifiable {
        /// Identifier of the entry that stayed behind.
        public let id: UUID
        /// Name shown in the UI.
        public let displayName: String
        /// Reader-facing explanation, already localised.
        public let reason: String

        public init(id: UUID, displayName: String, reason: String) {
            self.id = id
            self.displayName = displayName
            self.reason = reason
        }
    }

    /// Outcome of `migrateExternalCopies`.
    public struct MigrationReport: Equatable, Sendable {
        /// Entries whose copy now lives in the library folder.
        public let migrated: [LibraryItem]
        /// Entries that kept living outside it.
        public let issues: [MigrationIssue]

        public var isEmpty: Bool { migrated.isEmpty && issues.isEmpty }
    }

    /// Why one copy could not be moved into the library folder.
    public enum MigrationError: LocalizedError, Equatable {
        /// The copy is not where `library.json` says it is.
        case sourceMissing(String)
        /// An iCloud placeholder that has not been downloaded yet.
        case sourceNotDownloaded(String)
        /// `copyItem` refused; the payload is the system message.
        case copyFailed(String)
        /// The copy exists but does not open as a dictionary.
        case verificationFailed(String)

        public var errorDescription: String? {
            switch self {
            case .sourceMissing(let name):
                return "「\(name)」的原件已不在原位置，未能迁入词典库。"
            case .sourceNotDownloaded(let name):
                return "「\(name)」的原件是 iCloud 尚未下载的占位文件，请先在访达中下载后重启应用。"
            case .copyFailed(let detail):
                return "复制原件时出错：\(detail)"
            case .verificationFailed(let detail):
                return "复制出的副本无法打开：\(detail)"
            }
        }
    }

    /// True when the entry's copy already sits inside `directory`.
    public static func isStored(_ item: LibraryItem, in directory: URL) -> Bool {
        let root = directory.standardizedFileURL.path
        let copy = copyDirectory(for: item, in: directory).standardizedFileURL.path
        guard copy != root else { return false }
        let prefix = root.hasSuffix("/") ? root : root + "/"
        return copy.hasPrefix(prefix)
    }

    /// Copy every entry that still lives outside the library folder into it.
    ///
    /// Runs once per launch and is idempotent: an entry that already points
    /// into `directory` is left alone, and the copy keeps the entry's own
    /// identifier (falling back to a fresh one when that name is taken).
    /// The original file is only read, never moved or deleted.
    ///
    /// - Parameters:
    ///   - directory: Library folder (`Application Support/ODict`).
    ///   - state: Library index, updated in place for the entries that moved.
    ///   - verify: Opens a copy to prove it is usable. The default checks the
    ///     SQLite header; the app passes a real `OdcDictionary` open.
    public static func migrateExternalCopies(
        in directory: URL,
        state: inout LibraryState,
        verify: (URL) throws -> Void = verifyRepository,
        fileManager: FileManager = .default
    ) -> MigrationReport {
        let external = state.items.filter { !isStored($0, in: directory) }
        guard !external.isEmpty else {
            return MigrationReport(migrated: [], issues: [])
        }
        var migrated: [LibraryItem] = []
        var issues: [MigrationIssue] = []
        for item in external {
            do {
                let copy = try adopt(item, into: directory, verify: verify, fileManager: fileManager)
                replace(item.id, with: copy, in: &state)
                migrated.append(copy)
            } catch {
                issues.append(
                    MigrationIssue(
                        id: item.id,
                        displayName: item.displayName,
                        reason: error.localizedDescription
                    )
                )
            }
        }
        return MigrationReport(migrated: migrated, issues: issues)
    }

    /// Copy one entry into `directory` and return the updated entry.
    ///
    /// Keeps `item.id` as the stored name when free; otherwise throws away a
    /// stale half-copy (when it does not open) or takes a fresh identifier.
    public static func adopt(
        _ item: LibraryItem,
        into directory: URL,
        verify: (URL) throws -> Void = verifyRepository,
        fileManager: FileManager = .default
    ) throws -> LibraryItem {
        let source = copyDirectory(for: item, in: directory).standardizedFileURL
        // An empty or escaping `storedName` resolves to the library itself or a
        // parent of it; copying that would recurse. Treat it as a missing copy.
        let root = directory.standardizedFileURL.path
        guard source.path != root,
              !root.hasPrefix(source.path + "/"),
              fileManager.fileExists(atPath: source.path) else {
            throw MigrationError.sourceMissing(item.displayName)
        }
        try refuseUndownloadedPlaceholder(source, displayName: item.displayName)

        var storedName = "\(item.id.uuidString).odc"
        let preferred = directory.appending(path: storedName, directoryHint: .isDirectory)
        if fileManager.fileExists(atPath: preferred.path) {
            // An earlier run may have copied this entry and stopped before
            // saving the index. Reuse that copy when it still opens; otherwise
            // keep it untouched and write the new copy under a fresh name.
            if (try? verify(preferred)) != nil {
                return renamed(item, storedName: storedName)
            }
            storedName = "\(UUID().uuidString).odc"
        }

        let destination = directory.appending(path: storedName, directoryHint: .notDirectory)
        do {
            try copy(source, to: destination, fileManager: fileManager)
            try verify(destination)
        } catch {
            try? fileManager.removeItem(at: destination)
            throw error
        }
        return renamed(item, storedName: storedName)
    }

    /// Structural check used when the caller has no dictionary handle to open
    /// with: the copy holds an `odc.db` that starts with the SQLite header.
    public static func verifyRepository(_ url: URL) throws {
        let database = url.appending(path: "odc.db", directoryHint: .notDirectory)
        let handle: FileHandle
        do {
            handle = try FileHandle(forReadingFrom: database)
        } catch {
            throw MigrationError.verificationFailed(error.localizedDescription)
        }
        defer { try? handle.close() }
        let header = (try? handle.read(upToCount: 16)) ?? Data()
        guard header.count == 16,
              String(decoding: header.prefix(15), as: UTF8.self) == "SQLite format 3" else {
            throw MigrationError.verificationFailed("文件不是 SQLite 词典库")
        }
    }

    private static func copy(
        _ source: URL,
        to destination: URL,
        fileManager: FileManager
    ) throws {
        let values = try source.resourceValues(forKeys: [.isDirectoryKey])
        do {
            if values.isDirectory == true {
                try fileManager.copyItem(at: source, to: destination)
            } else {
                try fileManager.createDirectory(at: destination, withIntermediateDirectories: false)
                try fileManager.copyItem(
                    at: source,
                    to: destination.appending(path: "odc.db", directoryHint: .notDirectory)
                )
            }
        } catch {
            throw MigrationError.copyFailed(error.localizedDescription)
        }
    }

    /// Fail early, with a readable reason, on an iCloud file that was never
    /// downloaded: `copyItem` would otherwise block, or die with
    /// "Resource deadlock avoided" after the UI already started.
    private static func refuseUndownloadedPlaceholder(
        _ url: URL,
        displayName: String
    ) throws {
        let values = try? url.resourceValues(
            forKeys: [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey]
        )
        guard values?.isUbiquitousItem == true else { return }
        guard values?.ubiquitousItemDownloadingStatus != .current else { return }
        throw MigrationError.sourceNotDownloaded(displayName)
    }

    private static func renamed(_ item: LibraryItem, storedName: String) -> LibraryItem {
        var copy = item
        copy.storedName = storedName
        let stem = (storedName as NSString).deletingPathExtension
        if let id = UUID(uuidString: stem) {
            copy.id = id
        }
        return copy
    }

    private static func replace(_ id: UUID, with copy: LibraryItem, in state: inout LibraryState) {
        guard let index = state.items.firstIndex(where: { $0.id == id }) else { return }
        state.items[index] = copy
        if let openIndex = state.openItemIDs.firstIndex(of: id) {
            state.openItemIDs[openIndex] = copy.id
        }
        if state.lastOpenedID == id {
            state.lastOpenedID = copy.id
        }
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

/// Cheap, stable identity for a picked file.
///
/// Reading a whole 175 MB MDX just to compare it is wasteful, so the identity
/// is the file size plus SHA-256 of its first 16 MiB. Files smaller than that
/// are effectively hashed in full.
public enum LibraryFileFingerprint {
    /// Bytes hashed from the head of a file.
    public static let prefixBytes = 16 * 1024 * 1024

    public static func descriptor(
        for url: URL,
        accessingSecurityScope: Bool = false
    ) throws -> LibrarySourceDescriptor {
        let scoped = accessingSecurityScope ? url.startAccessingSecurityScopedResource() : false
        defer {
            if scoped {
                url.stopAccessingSecurityScopedResource()
            }
        }
        let values = try url.resourceValues(forKeys: [.isDirectoryKey, .fileSizeKey])
        if values.isDirectory == true {
            let manifest = try directoryManifest(of: url)
            return LibrarySourceDescriptor(
                name: url.lastPathComponent,
                bytes: manifest.bytes,
                contentHash: manifest.hash
            )
        }
        return LibrarySourceDescriptor(
            name: url.lastPathComponent,
            bytes: Int64(values.fileSize ?? 0),
            contentHash: try contentHash(of: url)
        )
    }

    /// SHA-256 over the first `limit` bytes of `url`, hex encoded.
    public static func contentHash(of url: URL, limit: Int = prefixBytes) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        var remaining = max(limit, 0)
        while remaining > 0 {
            let chunkSize = min(remaining, 1 << 20)
            guard let chunk = try handle.read(upToCount: chunkSize), !chunk.isEmpty else {
                break
            }
            hasher.update(data: chunk)
            remaining -= chunk.count
        }
        return hex(hasher.finalize())
    }

    /// Metadata-only signature for a repository directory (name plus size of
    /// every regular file), so picking a folder need not read its contents.
    private static func directoryManifest(of directory: URL) throws -> (bytes: Int64, hash: String) {
        let keys: Set<URLResourceKey> = [.isDirectoryKey, .fileSizeKey]
        let enumerator = FileManager.default.enumerator(
            at: directory,
            includingPropertiesForKeys: Array(keys),
            options: [.skipsHiddenFiles]
        )
        var entries: [String] = []
        var total: Int64 = 0
        while let entry = enumerator?.nextObject() as? URL {
            let values = try entry.resourceValues(forKeys: keys)
            guard values.isDirectory != true else { continue }
            let name = entry.path.replacingOccurrences(
                of: directory.path + "/",
                with: "",
                options: [.anchored]
            )
            let size = Int64(values.fileSize ?? 0)
            total += size
            entries.append("\(name)|\(size)")
        }
        entries.sort()
        var hasher = SHA256()
        for line in entries {
            hasher.update(data: Data(line.utf8))
        }
        return (total, hex(hasher.finalize()))
    }

    private static func hex(_ digest: SHA256.Digest) -> String {
        digest.map { String(format: "%02x", $0) }.joined()
    }
}
