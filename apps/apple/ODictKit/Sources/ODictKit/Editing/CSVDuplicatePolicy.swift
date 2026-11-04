import Foundation

/// Client-side sanity limits applied while building an import plan.
///
/// The core format has no hard field limit; these values only catch broken
/// spreadsheet cells (for example a whole sheet pasted into one cell) before
/// they reach the dictionary.
public enum CSVImportLimits {
    public static let maxKeyLength = 512
    public static let maxFieldLength = 32_768
}

/// What to do when an imported key already exists in the CSV or the library.
public enum CSVDuplicatePolicy: String, Codable, CaseIterable, Equatable, Sendable {
    /// Keep the existing entry and drop the incoming row.
    case skip
    /// Replace the existing entry with the incoming one.
    case overwrite
    /// Fill empty fields from the incoming entry and keep the rest.
    case merge

    public var title: String {
        switch self {
        case .skip: "跳过"
        case .overwrite: "覆盖"
        case .merge: "合并"
        }
    }

    public var explanation: String {
        switch self {
        case .skip: "保留库内原条目，CSV 该行跳过"
        case .overwrite: "用 CSV 整条替换库内条目"
        case .merge: "CSV 中非空字段覆盖，空字段保留库内值"
        }
    }
}

/// Duplicate keys found before an import starts.
public struct CSVDuplicateSummary: Equatable, Sendable {
    /// Keys that appear more than once inside the CSV.
    public let internalDuplicates: [String]
    /// Rows that would be dropped when the CSV repeats a key.
    public let internalExtraRows: Int
    /// Keys the dictionary already has.
    public let libraryConflicts: [String]

    public init(internalDuplicates: [String], internalExtraRows: Int, libraryConflicts: [String]) {
        self.internalDuplicates = internalDuplicates
        self.internalExtraRows = internalExtraRows
        self.libraryConflicts = libraryConflicts
    }

    public var isEmpty: Bool { internalDuplicates.isEmpty && libraryConflicts.isEmpty }

    /// Number of distinct keys that need a decision.
    public var affectedKeyCount: Int { internalDuplicates.count + libraryConflicts.count }

    /// Short lines such as `apple ×3` for the confirmation sheet.
    public func exampleLines(limit: Int = 5) -> [String] {
        var lines: [String] = []
        for key in internalDuplicates.prefix(limit) {
            lines.append("\(key)（CSV 内重复）")
        }
        for key in libraryConflicts.prefix(max(0, limit - lines.count)) {
            lines.append("\(key)（库内已存在）")
        }
        return lines
    }
}

/// Finds duplicate keys without writing anything.
@MainActor
public enum CSVDuplicateDetector {
    /// Scan the table for repeated keys and keys the library already has.
    ///
    /// Lookup failures (a missing entry) are treated as "not present", the same
    /// way the editor resolves keys; a repository-wide failure surfaces later
    /// as an upsert error.
    @MainActor
    public static func summary(
        table: CSVTable,
        mapping: CSVColumnMapping,
        editing: ODCEditing
    ) -> CSVDuplicateSummary {
        guard let keyColumn = mapping.key, table.header.indices.contains(keyColumn) else {
            return CSVDuplicateSummary(internalDuplicates: [], internalExtraRows: 0, libraryConflicts: [])
        }

        var counts: [String: Int] = [:]
        var order: [String] = []
        for row in table.rows {
            guard keyColumn < row.count else { continue }
            let key = row[keyColumn].trimmingCharacters(in: .whitespacesAndNewlines)
            guard !key.isEmpty else { continue }
            if counts[key] == nil {
                order.append(key)
            }
            counts[key, default: 0] += 1
        }

        let repeated = order.filter { (counts[$0] ?? 0) > 1 }
        let extraRows = repeated.reduce(0) { $0 + (counts[$1] ?? 1) - 1 }
        let conflicts = order.filter { key in
            (try? editing.entry(for: key)) != nil
        }

        return CSVDuplicateSummary(
            internalDuplicates: repeated,
            internalExtraRows: extraRows,
            libraryConflicts: conflicts
        )
    }

    /// Overlay the incoming entry onto the stored one: non-empty CSV fields win,
    /// empty CSV fields keep the library value.
    public static func merged(library: DictionaryEntry, csv: DictionaryEntry) -> DictionaryEntry {
        var extra = library.extra
        for (key, value) in csv.extra {
            extra[key] = value
        }
        // The plan falls back to the key when the display cell is empty, so a
        // display equal to the key means "no display was supplied".
        let display = csv.display == csv.key ? "" : csv.display
        return DictionaryEntry(
            key: csv.key,
            display: display.isEmpty ? library.display : display,
            phonetic: csv.phonetic.isEmpty ? library.phonetic : csv.phonetic,
            defs: csv.defs.isEmpty ? library.defs : csv.defs,
            rawHTML: csv.rawHTML ?? library.rawHTML,
            extra: extra
        )
    }
}

/// Remembers one duplicate policy per library.
///
/// The data is one short string, so UserDefaults is enough; the key mixes a
/// namespace prefix with the library identifier so two dictionaries never
/// share a choice.
@MainActor
public final class CSVDuplicatePolicyStore {
    public static let keyPrefix = "com.daxmate.odict.csv.duplicatePolicy."

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public func policy(for libraryKey: String) -> CSVDuplicatePolicy? {
        guard let raw = defaults.string(forKey: key(for: libraryKey)) else { return nil }
        return CSVDuplicatePolicy(rawValue: raw)
    }

    public func set(_ policy: CSVDuplicatePolicy, for libraryKey: String) {
        defaults.set(policy.rawValue, forKey: key(for: libraryKey))
    }

    public func clear(for libraryKey: String) {
        defaults.removeObject(forKey: key(for: libraryKey))
    }

    public func key(for libraryKey: String) -> String {
        Self.keyPrefix + libraryKey
    }
}
