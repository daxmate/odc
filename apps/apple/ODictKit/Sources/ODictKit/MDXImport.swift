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

/// Stage of a running conversion, mirroring `odc_import_progress_fn`.
public enum MDXImportStage: Int, Equatable, Sendable {
    case preparing = 0
    case entries = 1
    case resources = 2
    case committing = 3
    /// The core returned; the app is opening the converted document.
    case finished = 4

    public var title: String {
        switch self {
        case .preparing: return "准备"
        case .entries: return "导入词条"
        case .resources: return "导入资源"
        case .committing: return "写入 / 打包"
        case .finished: return "完成"
        }
    }
}

/// Overall progress of one conversion, ready for the banner.
public struct MDXImportProgress: Equatable, Sendable {
    /// Stage the sample belongs to.
    public let stage: MDXImportStage
    /// 0…1. Never decreases during one conversion.
    public let fraction: Double
    /// Stage name, e.g. 导入资源.
    public let title: String
    /// Counts under the bar, e.g. 资源 45,000 / 182,429 · 卷 2/2.
    public let detail: String
    /// Whether `fraction` measures a known total. False while the size of the
    /// job is unknown, which is when the banner falls back to a spinner.
    public let isDeterminate: Bool
    /// When the core handed the job over for packing, if it has.
    ///
    /// The final write has no sample of its own, so the banner counts seconds
    /// from this instant instead of inventing a percentage.
    public let committingSince: Date?

    public init(
        stage: MDXImportStage,
        fraction: Double,
        title: String,
        detail: String,
        isDeterminate: Bool,
        committingSince: Date? = nil
    ) {
        self.stage = stage
        self.fraction = fraction
        self.title = title
        self.detail = detail
        self.isDeterminate = isDeterminate
        self.committingSince = committingSince
    }

    /// Detail line plus the seconds spent writing the package.
    ///
    /// Returns `detail` unchanged outside the committing stage.
    public func elapsedDetail(at date: Date = Date()) -> String {
        guard stage == .committing, let committingSince else { return detail }
        let seconds = max(0, Int(date.timeIntervalSince(committingSince).rounded(.down)))
        return "\(detail) · 已用 \(seconds) 秒"
    }

    /// Whole percent, clamped to 0…100.
    public var percent: Int {
        Int((min(max(fraction, 0), 1) * 100).rounded())
    }

    /// Sample shown before the core reports anything.
    public static var preparingSample: MDXImportProgress {
        var tally = MDXImportProgressTally()
        return tally.apply(stage: .preparing, done: 0, total: 0, volumeIndex: 0, volumeCount: 0)
    }

    /// Sample shown once the core returned and the app takes over.
    public static var finishedSample: MDXImportProgress {
        var tally = MDXImportProgressTally()
        return tally.finish()
    }

    /// Coarse stage the older `progress:` callback used.
    public var phase: MDXImportPhase {
        switch stage {
        case .preparing: return .preparing
        case .entries, .resources, .committing: return .importing
        case .finished: return .finished
        }
    }
}

/// Adds stage-scoped samples up into one overall fraction.
///
/// The core reports `done` and `total` per stage; the banner needs one bar over
/// the whole job, so the entries and the resources are weighted by count.
public struct MDXImportProgressTally {
    private var entriesTotal: UInt64 = 0
    private var entriesDone: UInt64 = 0
    private var resourcesTotal: UInt64 = 0
    private var resourcesDone: UInt64 = 0
    private var lastFraction: Double = 0
    private var finished = false
    private var committingSince: Date?
    private let now: @Sendable () -> Date

    /// - Parameter now: Clock used to stamp the committing stage; injectable
    ///   so the elapsed-seconds line is testable.
    public init(now: @escaping @Sendable () -> Date = { Date() }) {
        self.now = now
    }

    /// Fold one core sample into the running total.
    public mutating func apply(
        stage: MDXImportStage,
        done: UInt64,
        total: UInt64,
        volumeIndex: UInt32,
        volumeCount: UInt32
    ) -> MDXImportProgress {
        switch stage {
        case .preparing:
            resourcesTotal = total
            resourcesDone = 0
            entriesDone = 0
        case .entries:
            entriesTotal = total
            entriesDone = min(done, total)
        case .resources:
            resourcesTotal = total
            resourcesDone = min(done, total)
            entriesDone = entriesTotal
        case .committing:
            entriesDone = entriesTotal
            resourcesDone = resourcesTotal
            // Stamped once: the core sends a single sample for this stage, but
            // a repeated one must not restart the clock.
            committingSince = committingSince ?? now()
        case .finished:
            entriesDone = entriesTotal
            resourcesDone = resourcesTotal
            finished = true
        }
        let grandTotal = entriesTotal + resourcesTotal
        let measured: Double
        if stage == .committing {
            // Everything the core measured is written; packing the container
            // is the caller's job and reports nothing. Hold the bar where the
            // writing stages left it rather than claiming 100%.
            measured = lastFraction
        } else if grandTotal == 0 {
            measured = finished ? 1 : 0
        } else {
            measured = Double(entriesDone + resourcesDone) / Double(grandTotal)
        }
        // The core throttles its samples but the UI must never move backwards.
        let fraction = max(lastFraction, min(max(measured, 0), 1))
        lastFraction = fraction
        return MDXImportProgress(
            stage: stage,
            fraction: fraction,
            title: stage.title,
            detail: Self.detail(
                stage: stage,
                done: done,
                total: total,
                volumeIndex: volumeIndex,
                volumeCount: volumeCount,
                entriesTotal: entriesTotal,
                resourcesTotal: resourcesTotal
            ),
            isDeterminate: grandTotal > 0 && stage != .committing,
            committingSince: committingSince
        )
    }

    /// Final sample once the core returned.
    public mutating func finish() -> MDXImportProgress {
        apply(stage: .finished, done: 0, total: 0, volumeIndex: 0, volumeCount: 0)
    }

    private static func detail(
        stage: MDXImportStage,
        done: UInt64,
        total: UInt64,
        volumeIndex: UInt32,
        volumeCount: UInt32,
        entriesTotal: UInt64,
        resourcesTotal: UInt64
    ) -> String {
        switch stage {
        case .preparing:
            return total > 0 ? "扫描到 \(grouped(total)) 个资源" : "正在准备…"
        case .entries:
            return "词条 \(grouped(min(done, total))) / \(grouped(total))"
        case .resources:
            // The media copy is the long part of an illustrated dictionary, so
            // the banner counts resources, not entries, while it runs.
            var text = "导入资源 \(grouped(min(done, total))) / \(grouped(total)) 个"
            if volumeCount > 0 {
                text += " · 卷 \(volumeIndex)/\(volumeCount)"
            }
            return text
        case .committing:
            return "正在写入词典包（\(contents(entriesTotal: entriesTotal, resourcesTotal: resourcesTotal))），请勿退出"
        case .finished:
            return ""
        }
    }

    /// What the package holds, for the committing line.
    private static func contents(entriesTotal: UInt64, resourcesTotal: UInt64) -> String {
        var parts: [String] = []
        if entriesTotal > 0 {
            parts.append("\(grouped(entriesTotal)) 条")
        }
        if resourcesTotal > 0 {
            parts.append("\(grouped(resourcesTotal)) 个资源")
        }
        return parts.isEmpty ? "词典数据" : parts.joined(separator: " + ")
    }

    /// Thousands separator, stable across locales so the tests and the UI agree.
    static func grouped(_ value: UInt64) -> String {
        let digits = Array(String(value))
        var text = ""
        for (index, digit) in digits.enumerated() {
            if index > 0 && (digits.count - index) % 3 == 0 {
                text.append(",")
            }
            text.append(digit)
        }
        return text
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

    /// One MDD volume that was imported.
    public struct MDDFile: Decodable, Equatable, Sendable {
        public let path: String
        public let entries: Int
        public let bytes: UInt64
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
    /// MDD volumes read next to the source, in import order.
    public let mddFiles: [MDDFile]
    /// Resources stored for the entries after the import.
    public let resourceCount: Int
    /// Payload bytes of those resources.
    public let resourceBytes: UInt64
    /// Non-fatal problems: unreadable or corrupt MDD volumes.
    public let warnings: [String]

    /// Keys the report uses; the MDD fields stay optional so a report from an
    /// older core still decodes.
    private enum CodingKeys: String, CodingKey {
        case importedEntries, written, removed, skippedBlocks, skippedEntries
        case corruptBlocks, deduplicated, skippedEmpty, skippedOversized
        case workingSet, scanWarning, header, outputBytes
        case mddFiles, resourceCount, resourceBytes, warnings
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        importedEntries = try container.decode(Int.self, forKey: .importedEntries)
        written = try container.decode(Int.self, forKey: .written)
        removed = try container.decode(Int.self, forKey: .removed)
        skippedBlocks = try container.decode(Int.self, forKey: .skippedBlocks)
        skippedEntries = try container.decode(Int.self, forKey: .skippedEntries)
        corruptBlocks = try container.decode([CorruptBlock].self, forKey: .corruptBlocks)
        deduplicated = try container.decode(Int.self, forKey: .deduplicated)
        skippedEmpty = try container.decode(Int.self, forKey: .skippedEmpty)
        skippedOversized = try container.decode(Int.self, forKey: .skippedOversized)
        workingSet = try container.decode(Int.self, forKey: .workingSet)
        scanWarning = try container.decodeIfPresent(String.self, forKey: .scanWarning)
        header = try container.decode(Header.self, forKey: .header)
        outputBytes = try container.decode(UInt64.self, forKey: .outputBytes)
        mddFiles = try container.decodeIfPresent([MDDFile].self, forKey: .mddFiles) ?? []
        resourceCount = try container.decodeIfPresent(Int.self, forKey: .resourceCount) ?? 0
        resourceBytes = try container.decodeIfPresent(UInt64.self, forKey: .resourceBytes) ?? 0
        warnings = try container.decodeIfPresent([String].self, forKey: .warnings) ?? []
    }

    /// One-line, user-facing summary of the conversion.
    public var summary: String {
        var parts = ["已导入 \(importedEntries) 条"]
        if skippedBlocks > 0 {
            parts.append("跳过 \(skippedBlocks) 个损坏块（\(skippedEntries) 条）")
        }
        if let scanWarning, !scanWarning.isEmpty {
            parts.append("⚠︎ \(scanWarning)")
        }
        if resourceCount > 0 {
            parts.append("资源 \(resourceCount) 个（\(Self.byteCount(resourceBytes))）")
        } else if !mddFiles.isEmpty {
            parts.append("未读到资源")
        }
        for warning in warnings {
            parts.append("⚠︎ \(warning)")
        }
        return parts.joined(separator: "，")
    }

    /// Human-readable byte count, e.g. "12.4 MB".
    static func byteCount(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
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
    /// The file lands in `documents` when the caller passes one (the library
    /// passes its staging folder) and in `importStagingDirectory` otherwise —
    /// never next to the source. Existing files are never replaced: the
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
        try await convert(source: source, documents: documents) { sample in
            progress?(sample.phase)
        }
    }

    /// Convert `source` on a background thread, reporting overall progress.
    ///
    /// Samples arrive on the importing thread, at most about ten times per
    /// second; hop to the main actor before touching the UI.
    public static func convert(
        source: URL,
        documents: URL? = nil,
        onProgress: (@Sendable (MDXImportProgress) -> Void)?
    ) async throws -> MDXImportResult {
        try await Task.detached(priority: .userInitiated) {
            try convertSynchronously(source: source, documents: documents, onProgress: onProgress)
        }.value
    }

    private static func convertSynchronously(
        source: URL,
        documents: URL?,
        onProgress: (@Sendable (MDXImportProgress) -> Void)?
    ) throws -> MDXImportResult {
        // The picker only grants the file the user chose. Sibling MDD volumes
        // need their own scope, which the importer holds for the whole run.
        var scopes: [(url: URL, active: Bool)] = []
        func claim(_ url: URL) -> URL {
            scopes.append((url, url.startAccessingSecurityScopedResource()))
            return url
        }
        defer {
            for scope in scopes where scope.active {
                scope.url.stopAccessingSecurityScopedResource()
            }
        }
        _ = claim(source)

        guard source.pathExtension.lowercased() == "mdx" else {
            throw MDXImportError.unsupportedSource(source.lastPathComponent)
        }
        let output = try outputURL(for: source, documents: documents)
        // The destination is only resolved by `outputURL`; create it here, once
        // there is something to write into it.
        try FileManager.default.createDirectory(
            at: output.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )

        let volumes = volumes(for: source).map(claim)
        onProgress?(MDXImportProgress.preparingSample)
        let report = try runCore(
            source: source,
            output: output,
            volumes: volumes,
            onProgress: onProgress
        )
        onProgress?(MDXImportProgress.finishedSample)
        return MDXImportResult(outputURL: output, report: report)
    }

    /// Carries the running total and the handler across the C ABI.
    ///
    /// The box is created on the importing thread, used only from the C
    /// callback on that thread and released when the core returns.
    private final class ProgressBox: @unchecked Sendable {
        var tally = MDXImportProgressTally()
        let handler: @Sendable (MDXImportProgress) -> Void

        init(handler: @escaping @Sendable (MDXImportProgress) -> Void) {
            self.handler = handler
        }
    }

    /// Runs `body` with a C string for `value`, or `nil` when it is absent.
    private static func withOptionalCString<Result>(
        _ value: String?,
        _ body: (UnsafePointer<CChar>?) -> Result
    ) -> Result {
        guard let value else { return body(nil) }
        return value.withCString { pointer in body(pointer) }
    }

    private static func runCore(
        source: URL,
        output: URL,
        volumes: [URL],
        onProgress: (@Sendable (MDXImportProgress) -> Void)? = nil
    ) throws -> MDXImportReport {
        var reportPointer: UnsafeMutablePointer<CChar>?
        var errorPointer: UnsafeMutablePointer<CChar>?
        let sourcePath = source.path
        let outputPath = output.path
        let volumeList = volumes.isEmpty ? nil : volumes.map(\.path)
        let volumeJSON = volumeList.flatMap { list -> Data? in
            try? JSONSerialization.data(withJSONObject: list, options: [])
        }
        // The C callback cannot capture Swift state, so the running total and
        // the handler travel through an Unmanaged box owned by this call.
        let box = onProgress.map(ProgressBox.init(handler:))
        let context = box.map { Unmanaged.passRetained($0).toOpaque() }
        defer {
            if let context {
                Unmanaged<ProgressBox>.fromOpaque(context).release()
            }
        }
        let callback: odc_import_progress_fn? = onProgress == nil
            ? nil
            : { stage, done, total, volumeIndex, volumeCount, context in
                guard let context else { return }
                let box = Unmanaged<ProgressBox>.fromOpaque(context).takeUnretainedValue()
                let sample = box.tally.apply(
                    stage: MDXImportStage(rawValue: Int(stage)) ?? .entries,
                    done: done,
                    total: total,
                    volumeIndex: volumeIndex,
                    volumeCount: volumeCount
                )
                box.handler(sample)
            }
        let code: Int32 = sourcePath.withCString { sourceCString in
            outputPath.withCString { outputCString in
                let json = volumeJSON.flatMap { String(data: $0, encoding: .utf8) }
                return withOptionalCString(json) { jsonCString in
                    odc_import_mdx_to_odc_with_mdd_progress(
                        sourceCString,
                        outputCString,
                        jsonCString,
                        callback,
                        context,
                        &reportPointer,
                        &errorPointer
                    )
                }
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


    /// The only `.mdx` at the top level of `directory`, when exactly one exists.
    ///
    /// Lets the file importer accept the dictionary folder itself, which is
    /// the only way a sandboxed app gets read access to the sibling `.mdd`
    /// volumes.
    public static func singleMDX(
        in directory: URL,
        fileManager: FileManager = .default
    ) -> URL? {
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles]
        ) else { return nil }
        let candidates = entries.filter { $0.pathExtension.lowercased() == "mdx" }
        return candidates.count == 1 ? candidates[0] : nil
    }

    /// MDD volumes that belong to `source`, in import order.
    ///
    /// The main volume (`<name>.mdd`) comes first, then the numbered ones in
    /// ascending numeric order, so `.10` sorts after `.2`. An unreadable
    /// directory (a sandbox without scope on the folder) simply yields no
    /// volumes; the core repeats the discovery when the list is empty.
    public static func volumes(
        for source: URL,
        fileManager: FileManager = .default
    ) -> [URL] {
        let stem = source.deletingPathExtension().lastPathComponent.lowercased()
        guard !stem.isEmpty else { return [] }
        let directory = source.deletingLastPathComponent()
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) else { return [] }
        var main: URL?
        var numbered: [(Int, URL)] = []
        for url in entries {
            guard let number = volumeNumber(of: url.lastPathComponent, stem: stem) else {
                continue
            }
            if number == 0 {
                main = main ?? url
            } else {
                numbered.append((number, url))
            }
        }
        numbered.sort { $0.0 < $1.0 }
        return (main.map { [$0] } ?? []) + numbered.map(\.1)
    }

    /// Volume number encoded in `name` for a dictionary called `stem`.
    private static func volumeNumber(of name: String, stem: String) -> Int? {
        let lowered = name.lowercased()
        guard lowered.hasSuffix(".mdd") else { return nil }
        let base = String(lowered.dropLast(4))
        if base == stem { return 0 }
        let prefix = stem + "."
        guard base.hasPrefix(prefix) else { return nil }
        let suffix = base.dropFirst(prefix.count)
        guard !suffix.isEmpty, suffix.allSatisfy(\.isNumber), let number = Int(suffix) else {
            return nil
        }
        return number
    }

    /// Folder a conversion is written to.
    ///
    /// Conversions live in the app's own storage. They used to be written next
    /// to the source `.mdx` whenever that folder was writable, which left a
    /// stray `<name>.odc` in the reader's own folder and made the same
    /// dictionary exist in two places. The library hands over its staging
    /// folder explicitly; anything else lands in `importStagingDirectory`.
    private static func destinationDirectory(
        documents: URL?,
        fileManager: FileManager
    ) -> URL {
        if let documents {
            return documents
        }
        return importStagingDirectory(fileManager: fileManager)
    }

    /// Default home for conversions the caller did not place itself:
    /// `<Application Support>/ODict/Imports`.
    ///
    /// Only the path is resolved here; `convert` creates the folder when it has
    /// something to write, so asking for a destination has no side effects.
    /// The user's Documents directory is a last resort for a container that has
    /// no Application Support at all.
    public static func importStagingDirectory(
        fileManager: FileManager = .default
    ) -> URL {
        if let support = try? fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: false
        ) {
            return support
                .appending(path: "ODict", directoryHint: .isDirectory)
                .appending(path: "Imports", directoryHint: .isDirectory)
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
