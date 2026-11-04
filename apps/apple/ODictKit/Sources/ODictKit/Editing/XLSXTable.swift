import CoreXLSX
import Foundation

/// A table parsed from the first non-empty worksheet of an `.xlsx` workbook.
///
/// The result deliberately reuses `CSVTable`: importing an Excel file goes
/// through the same column mapping, preview and write path as CSV, and only the
/// parsing step differs. Values are not type-inferred; every cell becomes a
/// string, numbers are kept verbatim as Excel stored them, booleans become
/// `TRUE` / `FALSE` and blanks become empty strings.
public struct XLSXTable: Equatable, Sendable {
    public let table: CSVTable
    /// Name of the worksheet that was read, when the workbook declares one.
    public let sheetName: String?
    /// Number of worksheets in the workbook, including empty ones.
    public let sheetCount: Int

    /// A one-line explanation shown in the import preview when the workbook
    /// has more than one worksheet and only the first non-empty one is used.
    public var note: String? {
        guard sheetCount > 1 else { return nil }
        let name = sheetName ?? "第一个工作表"
        return "该文件有 \(sheetCount) 个工作表，本次只导入「\(name)」。"
    }

    public init(contentsOf url: URL) throws {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            throw XLSXParseError.unreadableFile
        }

        let parsed = try Self.parse(data)
        self.table = parsed.table
        self.sheetName = parsed.sheetName
        self.sheetCount = parsed.sheetCount
    }

    public init(data: Data) throws {
        let parsed = try Self.parse(data)
        self.table = parsed.table
        self.sheetName = parsed.sheetName
        self.sheetCount = parsed.sheetCount
    }

    /// CoreXLSX reports unreadable archives with its own error type; every
    /// failure that is not ours becomes one readable Chinese message.
    private static func parse(_ data: Data) throws -> (table: CSVTable, sheetName: String?, sheetCount: Int) {
        do {
            return try read(data)
        } catch let error as XLSXParseError {
            throw error
        } catch {
            throw XLSXParseError.unreadableWorkbook
        }
    }

    private static func read(_ data: Data) throws -> (table: CSVTable, sheetName: String?, sheetCount: Int) {
        // `XLSXFile` is not `Sendable`; it stays local to this call and is never
        // shared, so the parse result can be handed to main-actor UI code.
        let file = try XLSXFile(data: data)
        guard let workbook = try file.parseWorkbooks().first else {
            throw XLSXParseError.missingWorksheet
        }

        let sheets = try file.parseWorksheetPathsAndNames(workbook: workbook)
        guard !sheets.isEmpty else {
            throw XLSXParseError.missingWorksheet
        }

        let sharedStrings = try file.parseSharedStrings()

        var selected: (name: String?, grid: [[String]])?
        for sheet in sheets {
            let worksheet = try file.parseWorksheet(at: sheet.path)
            let grid = Self.grid(of: worksheet, sharedStrings: sharedStrings)
            if grid.contains(where: { row in row.contains { !$0.isEmpty } }) {
                selected = (sheet.name, grid)
                break
            }
        }

        guard let selected else {
            throw XLSXParseError.emptyWorksheet
        }
        guard let header = selected.grid.first else {
            throw XLSXParseError.emptyWorksheet
        }
        guard header.contains(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) else {
            throw XLSXParseError.missingHeader
        }

        return (CSVTable(header: header, rows: Array(selected.grid.dropFirst())), selected.name, sheets.count)
    }

    /// Lays the worksheet out as a rectangular grid: missing cells and missing
    /// columns become empty strings, and rows without any cell are skipped.
    private static func grid(of worksheet: Worksheet, sharedStrings: SharedStrings?) -> [[String]] {
        var rows: [[Int: String]] = []
        var width = 0

        for row in worksheet.data?.rows ?? [] {
            guard !row.cells.isEmpty else { continue }

            var cells: [Int: String] = [:]
            for cell in row.cells {
                let column = columnIndex(of: cell.reference.column.value)
                width = max(width, column)
                let text = text(of: cell, sharedStrings: sharedStrings)
                if !text.isEmpty {
                    cells[column] = text
                }
            }
            rows.append(cells)
        }

        guard width > 0 else { return [] }
        return rows.map { cells in
            (1...width).map { cells[$0] ?? "" }
        }
    }

    /// One-based index for an Excel column reference such as `A` or `AB`.
    private static func columnIndex(of reference: String) -> Int {
        var index = 0
        for scalar in reference.uppercased().unicodeScalars
        where scalar.value >= 65 && scalar.value <= 90 {
            index = index * 26 + Int(scalar.value - 64)
        }
        return max(index, 1)
    }

    private static func text(of cell: Cell, sharedStrings: SharedStrings?) -> String {
        if let inline = cell.inlineString?.text {
            return inline
        }

        switch cell.type {
        case .sharedString?:
            guard
                let raw = cell.value,
                let position = Int(raw),
                let sharedStrings,
                sharedStrings.items.indices.contains(position)
            else { return cell.value ?? "" }

            let item = sharedStrings.items[position]
            if let text = item.text {
                return text
            }
            // Rich-text runs keep their fragments in `richText`.
            return item.richText.compactMap(\.text).joined()
        case .bool?:
            guard let raw = cell.value else { return "" }
            return raw == "1" ? "TRUE" : "FALSE"
        default:
            return cell.value ?? ""
        }
    }
}

public enum XLSXParseError: LocalizedError, Equatable {
    case unreadableFile
    case unreadableWorkbook
    case missingWorksheet
    case emptyWorksheet
    case missingHeader

    public var errorDescription: String? {
        switch self {
        case .unreadableFile:
            return "无法读取这个文件。"
        case .unreadableWorkbook:
            return "无法读取这个 Excel 文件，它可能已损坏、已加密，或不是 .xlsx 格式。"
        case .missingWorksheet:
            return "这个 Excel 文件里没有工作表。"
        case .emptyWorksheet:
            return "这个 Excel 文件的工作表都是空的。"
        case .missingHeader:
            return "Excel 的第一个工作表必须以表头开始。"
        }
    }
}
