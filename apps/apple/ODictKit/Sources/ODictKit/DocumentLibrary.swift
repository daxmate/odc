import Foundation

public enum DocumentLibraryError: LocalizedError {
    case unsupportedItem(String)
    case copyFailed(String)

    public var errorDescription: String? {
        switch self {
        case .unsupportedItem(let name):
            return "\(name) 既不是 ODC 仓库目录，也不是 .odc 文件。"
        case .copyFailed(let detail):
            return "无法把词典复制到沙盒：\(detail)"
        }
    }
}

/// Copies picked documents into the app's own storage before opening them.
///
/// The C ABI opens a repository *directory* that contains `odc.db`. The
/// published single-file `.odc` container is a bare SQLite database, so a
/// picked file is copied into a fresh directory as `odc.db` before opening.
/// Copying also avoids reopening the user's file inside the sandbox and keeps
/// security-scoped access short-lived.
public enum DocumentLibrary {
    /// Directory that holds imported copies.
    public static func applicationSupportContainer() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        return try libraryContainer(in: support)
    }

    /// Ensure that `<support>/ODict` exists and return it.
    ///
    /// On a fresh install (or after the user clears Application Support)
    /// `FileManager` creates Application Support but not the app's own
    /// subfolder. Callers such as `LibraryStore.orphanCopy(in:…)` enumerate
    /// that folder straight away, so opening a dictionary on first launch
    /// failed with "no such file". Creating it here — idempotently — removes
    /// the dependency on an earlier import having made the directory.
    static func libraryContainer(in support: URL) throws -> URL {
        let container = support.appending(path: "ODict", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        return container
    }

    /// Copy the picked URL into a fresh `<uuid>.odc` entry and return it.
    ///
    /// - Parameter container: Override for tests; defaults to Application Support.
    public static func copyDocument(at url: URL, into container: URL? = nil) throws -> URL {
        let scoped = url.startAccessingSecurityScopedResource()
        defer {
            if scoped {
                url.stopAccessingSecurityScopedResource()
            }
        }

        let fileManager = FileManager.default
        let root = try container ?? applicationSupportContainer()
        try fileManager.createDirectory(at: root, withIntermediateDirectories: true)

        let values = try url.resourceValues(forKeys: [.isDirectoryKey, .isRegularFileKey])
        let destination = root.appending(path: "\(UUID().uuidString).odc", directoryHint: .notDirectory)

        do {
            if values.isDirectory == true {
                try fileManager.copyItem(at: url, to: destination)
                return destination
            }
            guard values.isRegularFile == true else {
                throw DocumentLibraryError.unsupportedItem(url.lastPathComponent)
            }
            // The parent container is created above; a pre-existing destination
            // entry must still fail loudly rather than be merged into.
            try fileManager.createDirectory(at: destination, withIntermediateDirectories: false)
            let database = destination.appending(path: "odc.db", directoryHint: .notDirectory)
            try fileManager.copyItem(at: url, to: database)
            return destination
        } catch let error as DocumentLibraryError {
            throw error
        } catch {
            try? fileManager.removeItem(at: destination)
            throw DocumentLibraryError.copyFailed(error.localizedDescription)
        }
    }

    /// Remove an imported copy. Missing paths are ignored.
    public static func removeCopy(at url: URL) {
        try? FileManager.default.removeItem(at: url)
    }
}
