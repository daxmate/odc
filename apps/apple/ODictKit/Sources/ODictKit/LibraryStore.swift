import CryptoKit
import Foundation

/// One dictionary the app has copied into its own storage.
///
/// `library.json` holds one of these per import so a relaunch can reopen the
/// copy instead of asking the reader to pick the file again.
public struct LibraryItem: Codable, Identifiable, Sendable, Equatable {
    /// Identifier of the copy: the `<uuid>.odc` directory name without extension.
    public let id: UUID
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
    public var lastOpenedID: UUID?

    public init(
        items: [LibraryItem] = [],
        lastOpenedID: UUID? = nil,
        version: Int = LibraryState.currentVersion
    ) {
        self.version = version
        self.items = items
        self.lastOpenedID = lastOpenedID
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
        let match = state.items.firstIndex { $0.id == item.id || $0.matches(item) }
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
        if state.lastOpenedID == id {
            state.lastOpenedID = nil
        }
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
        for entry in entries.sorted(by: { $0.lastPathComponent < $1.lastPathComponent })
        where entry.pathExtension == "odc" {
            let storedName = entry.lastPathComponent
            if state.items.contains(where: { $0.storedName == storedName }) {
                continue
            }
            let database = entry.appending(path: "odc.db", directoryHint: .notDirectory)
            guard fileManager.fileExists(atPath: database.path) else { continue }
            let descriptor = try LibraryFileFingerprint.descriptor(for: database)
            guard descriptor.bytes == source.bytes, descriptor.contentHash == hash else {
                continue
            }
            return LibraryItem(
                id: UUID(uuidString: entry.deletingPathExtension().lastPathComponent) ?? UUID(),
                displayName: source.name,
                storedName: storedName,
                sourceName: source.name,
                sourceBytes: source.bytes,
                sourceHash: hash
            )
        }
        return nil
    }

    /// The entry for the copy stored under `storedName`.
    public static func item(forStoredName storedName: String, in state: LibraryState) -> LibraryItem? {
        state.items.first { $0.storedName == storedName }
    }

    /// Directory of the copy, e.g. `<library>/<uuid>.odc`.
    public static func copyDirectory(for item: LibraryItem, in root: URL) -> URL {
        root.appending(path: item.storedName, directoryHint: .isDirectory)
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
