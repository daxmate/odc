import Foundation
import ODictCore

/// A table parsed from RFC 4180 CSV text.
///
/// The parser is deliberately small: it understands quoted fields, escaped
/// quotes, embedded commas and newlines, CRLF line endings and a leading BOM.
/// It does not infer types; every cell stays a string.
public struct CSVTable: Equatable, Sendable {
    public let header: [String]
    public let rows: [[String]]

    public init(header: [String], rows: [[String]]) {
        self.header = header
        self.rows = rows
    }

    public init(parsing text: String) throws {
        var characters = Array(text)
        if characters.first == "\u{FEFF}" {
            characters.removeFirst()
        }
        guard !characters.isEmpty else {
            throw CSVParseError.emptyFile
        }

        var records: [[String]] = []
        var row: [String] = []
        var field = ""
        var inQuotes = false
        var index = 0

        while index < characters.count {
            let character = characters[index]

            if inQuotes {
                if character == "\"" {
                    let next = index + 1
                    if next < characters.count, characters[next] == "\"" {
                        field.append("\"")
                        index += 2
                        continue
                    }
                    inQuotes = false
                    index += 1
                    continue
                }
                field.append(character)
                index += 1
                continue
            }

            switch character {
            case "\"":
                if field.isEmpty {
                    inQuotes = true
                } else {
                    // Lenient: a quote inside an unquoted field is data.
                    field.append(character)
                }
            case ",":
                row.append(field)
                field = ""
            case "\r":
                // A bare CR is a record break.
                row.append(field)
                field = ""
                records.append(row)
                row = []
            case "\r\n", "\n":
                // Swift folds CRLF into one grapheme cluster, so both forms
                // have to be named explicitly.
                row.append(field)
                field = ""
                records.append(row)
                row = []
            default:
                field.append(character)
            }
            index += 1
        }

        if inQuotes {
            throw CSVParseError.unterminatedQuote(record: records.count + 1)
        }

        if !field.isEmpty || !row.isEmpty {
            row.append(field)
            records.append(row)
        }

        guard let header = records.first, !header.isEmpty else {
            throw CSVParseError.missingHeader
        }
        guard header.contains(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) else {
            throw CSVParseError.missingHeader
        }

        self.header = header
        self.rows = Array(records.dropFirst())
    }
}

public enum CSVParseError: LocalizedError, Equatable {
    case emptyFile
    case missingHeader
    case unterminatedQuote(record: Int)

    public var errorDescription: String? {
        switch self {
        case .emptyFile:
            return "CSV 文件是空的。"
        case .missingHeader:
            return "CSV 第一行必须是表头。"
        case .unterminatedQuote(let record):
            return "第 \(record) 条记录里有未闭合的引号。"
        }
    }
}

/// Which CSV column feeds each entry field. `key` is required.
public struct CSVColumnMapping: Codable, Equatable, Sendable {
    public var key: Int?
    public var display: Int?
    public var phonetic: Int?
    public var pos: Int?
    public var sense: Int?
    public var examples: Int?
    public var labels: Int?

    public init(
        key: Int? = nil,
        display: Int? = nil,
        phonetic: Int? = nil,
        pos: Int? = nil,
        sense: Int? = nil,
        examples: Int? = nil,
        labels: Int? = nil
    ) {
        self.key = key
        self.display = display
        self.phonetic = phonetic
        self.pos = pos
        self.sense = sense
        self.examples = examples
        self.labels = labels
    }

    public var isValid: Bool { key != nil }

    /// Match common header spellings, case-insensitively.
    public static func suggested(for header: [String]) -> CSVColumnMapping {
        func index(of names: [String]) -> Int? {
            header.firstIndex { cell in
                names.contains(cell.trimmingCharacters(in: .whitespaces).lowercased())
            }
        }

        return CSVColumnMapping(
            key: index(of: ["key", "word", "headword"]),
            display: index(of: ["display", "headword", "title"]),
            phonetic: index(of: ["phonetic", "pronunciation", "ipa"]),
            pos: index(of: ["pos", "part of speech"]),
            sense: index(of: ["sense", "definition", "def"]),
            examples: index(of: ["examples", "example"]),
            labels: index(of: ["labels", "label"])
        )
    }
}

/// One row that could not become an entry.
public struct CSVRowIssue: Equatable, Sendable, Identifiable {
    /// One-based row number in the sheet; the header is row 1.
    public let row: Int
    /// Header of the column the issue belongs to, when there is one.
    public let column: String?
    public let reason: String

    public var id: String { "\(row)|\(column ?? "")|\(reason)" }

    public init(row: Int, column: String? = nil, reason: String) {
        self.row = row
        self.column = column
        self.reason = reason
    }

    /// One line for the report list and the clipboard.
    public var displayText: String {
        if let column, !column.isEmpty {
            return "第 \(row) 行 · 列「\(column)」：\(reason)"
        }
        return "第 \(row) 行：\(reason)"
    }
}

/// A merge of CSV rows into entries, ready to be written to the working set.
public struct CSVImportPlan: Sendable {
    public struct Group: Sendable {
        public let entry: DictionaryEntry
        /// Data rows that contributed to this entry (including skipped defs).
        public let sourceRows: [Int]

        public var rowCount: Int { sourceRows.count }

        public init(entry: DictionaryEntry, sourceRows: [Int]) {
            self.entry = entry
            self.sourceRows = sourceRows
        }
    }

    public let groups: [Group]
    public let totalRows: Int
    public let issues: [CSVRowIssue]

    public var entries: [DictionaryEntry] { groups.map(\.entry) }

    public init(groups: [Group], totalRows: Int, issues: [CSVRowIssue]) {
        self.groups = groups
        self.totalRows = totalRows
        self.issues = issues
    }

    /// Build entries from a parsed table.
    ///
    /// `duplicatePolicy` decides what happens when several rows share a key:
    /// `merge` (the default) folds them into one entry, `skip` keeps the first
    /// row and reports the rest, `overwrite` keeps the last row.
    public static func make(
        table: CSVTable,
        mapping: CSVColumnMapping,
        duplicatePolicy: CSVDuplicatePolicy = .merge
    ) -> CSVImportPlan? {
        guard let keyColumn = mapping.key, table.header.indices.contains(keyColumn) else {
            return nil
        }

        struct Draft {
            var key: String
            var display: String
            var phonetic: [String] = []
            var defs: [Definition] = []
            var rows: [Int] = []
        }

        var order: [String] = []
        var drafts: [String: Draft] = [:]
        var issues: [CSVRowIssue] = []

        func cell(_ row: [String], _ column: Int?) -> String {
            guard let column, row.indices.contains(column) else { return "" }
            return row[column]
        }

        func columnName(_ column: Int?) -> String? {
            guard let column, table.header.indices.contains(column) else { return nil }
            let name = table.header[column].trimmingCharacters(in: .whitespacesAndNewlines)
            return name.isEmpty ? nil : name
        }

        let keyName = columnName(keyColumn)

        for (offset, row) in table.rows.enumerated() {
            let rowNumber = offset + 2
            if row.allSatisfy({ $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) {
                issues.append(CSVRowIssue(row: rowNumber, reason: "空行"))
                continue
            }

            let key = cell(row, keyColumn).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !key.isEmpty else {
                issues.append(CSVRowIssue(row: rowNumber, column: keyName, reason: "key 为空"))
                continue
            }
            guard key.count <= CSVImportLimits.maxKeyLength else {
                issues.append(
                    CSVRowIssue(
                        row: rowNumber,
                        column: keyName,
                        reason: "key 超过 \(CSVImportLimits.maxKeyLength) 个字符"
                    )
                )
                continue
            }
            if let overflow = Self.overflowingColumn(in: row, mapping: mapping) {
                issues.append(
                    CSVRowIssue(
                        row: rowNumber,
                        column: columnName(overflow),
                        reason: "字段超过 \(CSVImportLimits.maxFieldLength) 个字符"
                    )
                )
                continue
            }

            let display = cell(row, mapping.display).trimmingCharacters(in: .whitespacesAndNewlines)
            let phonetic = Self.splitList(cell(row, mapping.phonetic), separators: ["|"])
            let pos = cell(row, mapping.pos).trimmingCharacters(in: .whitespacesAndNewlines)
            let sense = cell(row, mapping.sense).trimmingCharacters(in: .whitespacesAndNewlines)
            let examples = Self.splitList(cell(row, mapping.examples), separators: ["\n", "|"])
            let labels = Self.splitList(cell(row, mapping.labels), separators: [",", ";"])

            let isDuplicate = drafts[key] != nil
            if isDuplicate {
                switch duplicatePolicy {
                case .skip:
                    issues.append(
                        CSVRowIssue(row: rowNumber, column: keyName, reason: "重复 key「\(key)」，已跳过")
                    )
                    continue
                case .overwrite:
                    drafts[key] = nil
                case .merge:
                    break
                }
            }

            if drafts[key] == nil {
                drafts[key] = Draft(key: key, display: display)
                if !isDuplicate {
                    order.append(key)
                }
            } else if drafts[key]?.display.isEmpty == true, !display.isEmpty {
                drafts[key]?.display = display
            }

            drafts[key]?.rows.append(rowNumber)
            drafts[key]?.phonetic.append(contentsOf: phonetic)

            let hasDefinition = !pos.isEmpty || !sense.isEmpty || !examples.isEmpty || !labels.isEmpty
            if hasDefinition {
                drafts[key]?.defs.append(
                    Definition(pos: pos, sense: sense, examples: examples, labels: labels)
                )
            }
        }

        var groups: [Group] = []
        groups.reserveCapacity(order.count)
        for key in order {
            guard let draft = drafts[key] else { continue }
            var seen = Set<String>()
            let phonetic = draft.phonetic.filter { seen.insert($0).inserted }
            let entry = DictionaryEntry(
                key: draft.key,
                display: draft.display.isEmpty ? draft.key : draft.display,
                phonetic: phonetic,
                defs: draft.defs,
                rawHTML: nil,
                extra: [:]
            )
            groups.append(Group(entry: entry, sourceRows: draft.rows))
        }

        return CSVImportPlan(groups: groups, totalRows: table.rows.count, issues: issues)
    }

    /// Index of the first mapped cell longer than the sanity limit.
    private static func overflowingColumn(in row: [String], mapping: CSVColumnMapping) -> Int? {
        let columns = [
            mapping.key, mapping.display, mapping.phonetic,
            mapping.pos, mapping.sense, mapping.examples, mapping.labels,
        ]
        for column in columns.compactMap({ $0 }).sorted() where row.indices.contains(column) {
            if row[column].count > CSVImportLimits.maxFieldLength {
                return column
            }
        }
        return nil
    }

    private static func splitList(_ text: String, separators: [Character]) -> [String] {
        var values: [String] = []
        var current = ""
        for character in text {
            if separators.contains(character) {
                let trimmed = current.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty {
                    values.append(trimmed)
                }
                current = ""
            } else {
                current.append(character)
            }
        }
        let trimmed = current.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            values.append(trimmed)
        }
        return values
    }
}
