import Foundation
import ODCFFI

/// Errors raised while converting an MDX dictionary into an `.odc` document.
public enum MDXImportError: LocalizedError, Equatable {
    /// The picked file is not an `.mdx` file.
    case unsupportedSource(String)
    /// No writable directory could hold the converted file.
    case noDestination(String)
    /// The core rejected the file.
    case conversionFailed(String)
    /// The core reported success without a report payload.
    case missingReport
    /// The report payload could not be decoded.
    case invalidReport(String)

    public var errorDescription: String? {
        switch self {
        case .unsupportedSource(let name):
            return "\(name) 不是 MDX 词典文件。"
        case .noDestination(let path):
            return "无法写入转换结果：没有可写的目录（\(path)）。"
        case .conversionFailed(let detail):
            return "导入 MDX 失败：\(detail)"
        case .missingReport:
            return "导入 MDX 失败：核心没有返回报告。"
        case .invalidReport(let detail):
            return "导入 MDX 失败：报告无法解析（\(detail)）。"
        }
    }
}

/// Stage of a running conversion.
///
/// The core converts and packs in a single call, so there is no observable
/// boundary between importing and packing; the app therefore reports the
/// stages it can honestly observe and never invents a percentage.
public enum MDXImportPhase: Equatable, Sendable {
    case preparing
    case importing
    case finished

    public var title: String {
        switch self {
        case .preparing:
            return "准备…"
        case .importing:
            return "导入中（含打包）…"
        case .finished:
            return "完成"
        }
    }
}

/// Outcome of converting one MDX file.
public struct MDXImportResult: Equatable, Sendable {
    /// Where the converted document was written.
    public let outputURL: URL
    /// What the core reported while converting.
    public let report: MDXImportReport

    public init(outputURL: URL, report: MDXImportReport) {
        self.outputURL = outputURL
        self.report = report
    }
}

/// Decoded form of the JSON report returned by `odc_import_mdx_to_odc`.
public struct MDXImportReport: Decodable, Equatable, Sendable {
    /// Header facts read from the source file.
    public struct Header: Decodable, Equatable, Sendable {
        public let engineVersion: String
        public let requiredEngineVersion: String
        public let encoding: String
        public let encrypted: String?
        public let records: UInt64
    }

    /// One record block the importer refused to read.
    public struct CorruptBlock: Decodable, Equatable, Sendable {
        public let ordinal: UInt64
        public let fileOffset: UInt64
        public let compressedSize: UInt64
        public let declaredSize: UInt64
        public let decompressedSize: UInt64?
        public let decompressedOffset: UInt64
        public let detail: String
    }

    public let importedEntries: Int
    public let written: Int
    public let removed: Int
    public let skippedBlocks: Int
    public let skippedEntries: Int
    public let corruptBlocks: [CorruptBlock]
    public let deduplicated: Int
    public let skippedEmpty: Int
    public let skippedOversized: Int
    public let workingSet: Int
    public let scanWarning: String?
    public let header: Header
    public let outputBytes: UInt64

    /// One-line, user-facing summary of the conversion.
    public var summary: String {
        var parts = ["已导入 \(importedEntries) 条"]
        if skippedBlocks > 0 {
            parts.append("跳过 \(skippedBlocks) 个损坏块（\(skippedEntries) 条）")
        }
        if let scanWarning, !scanWarning.isEmpty {
            parts.append("⚠︎ \(scanWarning)")
        }
        return parts.joined(separator: "，")
    }

    /// Decode a report produced by the C ABI.
    public static func decode(from data: Data) throws -> MDXImportReport {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        do {
            return try decoder.decode(MDXImportReport.self, from: data)
        } catch {
            throw MDXImportError.invalidReport(String(describing: error))
        }
    }
}

/// Converts MDX dictionaries into standalone `.odc` documents.
public enum MDXImporter {
    /// Destination for the converted copy of `source`.
    ///
    /// The file lands next to the source as `<name>.odc`. When that directory
    /// is not writable it falls back to `documents` (the caller's Documents
    /// directory by default). Existing files are never replaced: the
    /// `-1`, `-2`, … suffixes keep every conversion.
    public static func outputURL(
        for source: URL,
        documents: URL? = nil,
        fileManager: FileManager = .default
    ) throws -> URL {
        let base = source.deletingPathExtension().lastPathComponent
        guard !base.isEmpty else {
            throw MDXImportError.unsupportedSource(source.lastPathComponent)
        }
        let directory = destinationDirectory(
            for: source,
            documents: documents,
            fileManager: fileManager
        )
        return try uniqueURL(in: directory, base: base, fileManager: fileManager)
    }

    /// Convert `source` on a background thread and hand back the report.
    public static func convert(
        source: URL,
        documents: URL? = nil,
        progress: (@Sendable (MDXImportPhase) -> Void)? = nil
    ) async throws -> MDXImportResult {
        try await Task.detached(priority: .userInitiated) {
            try convertSynchronously(source: source, documents: documents, progress: progress)
        }.value
    }

    private static func convertSynchronously(
        source: URL,
        documents: URL?,
        progress: (@Sendable (MDXImportPhase) -> Void)?
    ) throws -> MDXImportResult {
        progress?(.preparing)
        let scoped = source.startAccessingSecurityScopedResource()
        defer {
            if scoped {
                source.stopAccessingSecurityScopedResource()
            }
        }

        guard source.pathExtension.lowercased() == "mdx" else {
            throw MDXImportError.unsupportedSource(source.lastPathComponent)
        }
        let output = try outputURL(for: source, documents: documents)

        progress?(.importing)
        let report = try runCore(source: source, output: output)
        progress?(.finished)
        return MDXImportResult(outputURL: output, report: report)
    }

    private static func runCore(source: URL, output: URL) throws -> MDXImportReport {
        var reportPointer: UnsafeMutablePointer<CChar>?
        var errorPointer: UnsafeMutablePointer<CChar>?
        let sourcePath = source.path
        let outputPath = output.path
        let code = sourcePath.withCString { sourceCString in
            outputPath.withCString { outputCString in
                odc_import_mdx_to_odc(sourceCString, outputCString, &reportPointer, &errorPointer)
            }
        }

        let failure = errorPointer.map { pointer -> String in
            defer { odc_string_free(pointer) }
            return String(cString: pointer)
        }
        guard code == 0 else {
            if let reportPointer {
                odc_string_free(reportPointer)
            }
            throw MDXImportError.conversionFailed(failure ?? "未知错误")
        }
        guard let reportPointer else {
            throw MDXImportError.missingReport
        }
        defer { odc_string_free(reportPointer) }
        let data = Data(bytes: reportPointer, count: strlen(reportPointer))
        return try MDXImportReport.decode(from: data)
    }

    private static func destinationDirectory(
        for source: URL,
        documents: URL?,
        fileManager: FileManager
    ) -> URL {
        let sibling = source.deletingLastPathComponent()
        if fileManager.isWritableFile(atPath: sibling.path) {
            return sibling
        }
        if let documents {
            return documents
        }
        if let documents = fileManager.urls(for: .documentDirectory, in: .userDomainMask).first {
            return documents
        }
        #if os(macOS)
        return fileManager.homeDirectoryForCurrentUser
            .appending(path: "Documents", directoryHint: .isDirectory)
        #else
        return URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
            .appending(path: "Documents", directoryHint: .isDirectory)
        #endif
    }

    private static func uniqueURL(
        in directory: URL,
        base: String,
        fileManager: FileManager
    ) throws -> URL {
        for index in 0...999 {
            let suffix = index == 0 ? "" : "-\(index)"
            let candidate = directory.appending(
                path: "\(base)\(suffix).odc",
                directoryHint: .notDirectory
            )
            if !fileManager.fileExists(atPath: candidate.path) {
                return candidate
            }
        }
        throw MDXImportError.noDestination(directory.path)
    }
}
