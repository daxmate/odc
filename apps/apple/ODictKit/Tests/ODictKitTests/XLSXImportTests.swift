import Foundation
import Testing
@testable import ODictKit

@Suite("XLSX parsing")
struct XLSXParsingTests {
    @Test("reads shared strings, numbers, inline strings and sparse rows")
    func readsSampleWorkbook() throws {
        let workbook = try XLSXTable(contentsOf: Fixture.sampleXLSX)
        let table = workbook.table

        #expect(workbook.sheetName == "Words")
        #expect(workbook.sheetCount == 1)
        #expect(workbook.note == nil)
        #expect(table.header == ["key", "display", "pos", "sense", "examples"])
        #expect(table.rows.count == 4)

        #expect(table.rows[0] == ["apple", "Apple", "noun", "a fruit", "an apple a day"])
        // Number cell, inline string, and a column that the row never defines.
        #expect(table.rows[1] == ["bank", "Bank", "42", "the side of a river", ""])
        // Sparse row: B, D and E are padded.
        #expect(table.rows[2] == ["pear", "", "Pear", "", ""])
        // Numeric key and a boolean cell.
        #expect(table.rows[3] == ["2026", "TRUE", "", "", ""])

        // The existing column heuristics and plan builder keep working on it.
        let mapping = CSVColumnMapping.suggested(for: table.header)
        #expect(mapping.key == 0)
        #expect(mapping.display == 1)
        let plan = try #require(CSVImportPlan.make(table: table, mapping: mapping))
        #expect(plan.totalRows == 4)
    }

    @Test("uses the first non-empty worksheet and explains the rest")
    func picksFirstNonEmptySheet() throws {
        let workbook = try XLSXTable(contentsOf: Fixture.multiSheetXLSX)

        #expect(workbook.sheetName == "Data")
        #expect(workbook.sheetCount == 3)
        #expect(workbook.table.header == ["key", "display"])
        #expect(workbook.table.rows == [["cat", "Cat"], ["dog", "Dog"]])

        let note = try #require(workbook.note)
        #expect(note.contains("3 个工作表"))
        #expect(note.contains("Data"))
    }

    @Test("rejects a worksheet without header text")
    func rejectsMissingHeader() {
        #expect(throws: XLSXParseError.missingHeader) {
            try XLSXTable(contentsOf: Fixture.blankHeaderXLSX)
        }
    }

    @Test("rejects a workbook without any cell")
    func rejectsEmptyWorkbook() {
        #expect(throws: XLSXParseError.emptyWorksheet) {
            try XLSXTable(contentsOf: Fixture.emptyXLSX)
        }
    }

    @Test("rejects a file that is not an archive")
    func rejectsBrokenFile() {
        #expect(throws: XLSXParseError.unreadableWorkbook) {
            try XLSXTable(contentsOf: Fixture.brokenXLSX)
        }
    }

    @Test("data initializer accepts the same workbook")
    func parsesFromData() throws {
        let data = try Data(contentsOf: Fixture.sampleXLSX)
        let workbook = try XLSXTable(data: data)
        #expect(workbook.table.header.first == "key")
    }
}

@Suite("XLSX import model")
struct XLSXImportModelTests {
    @Test("loading an xlsx fills the shared CSV import pipeline")
    @MainActor
    func loadFillsCSVImportModel() throws {
        let model = CSVImportModel(editing: FakeEditing(entries: []))

        model.load(url: Fixture.multiSheetXLSX)

        #expect(model.table?.header == ["key", "display"])
        #expect(model.fileName == "multi-sheet.xlsx")
        #expect(model.mapping.key == 0)
        #expect(model.canImport)
        #expect(model.sourceNote?.contains("Data") == true)
        #expect(model.alertMessage == nil)
    }

    @Test("loading a broken xlsx reports a readable error")
    @MainActor
    func loadReportsBrokenFile() {
        let model = CSVImportModel(editing: FakeEditing(entries: []))

        model.load(url: Fixture.brokenXLSX)

        #expect(model.table == nil)
        #expect(model.sourceNote == nil)
        #expect(model.alertMessage == XLSXParseError.unreadableWorkbook.errorDescription)
    }
}
