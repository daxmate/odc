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
        return support.appending(path: "ODict", directoryHint: .isDirectory)
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
