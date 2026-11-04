import Foundation
import Testing
@testable import ODictKit

@Suite("CSV parsing")
struct CSVParsingTests {
    @Test("handles quoted commas, newlines and escaped quotes")
    func quotedFields() throws {
        let text = "key,display,sense\n\"apple, green\",\"Apple\",\"a \"\"fruit\"\"\"\n\"pear\",Pear,\"line one\nline two\"\n"
        let table = try CSVTable(parsing: text)
        #expect(table.header == ["key", "display", "sense"])
        #expect(table.rows.count == 2)
        #expect(table.rows[0] == ["apple, green", "Apple", "a \"fruit\""])
        #expect(table.rows[1][2] == "line one\nline two")
    }

    @Test("strips the BOM and accepts CRLF")
    func bomAndCRLF() throws {
        let text = "\u{FEFF}key,display\r\napple,Apple\r\npear,Pear\r\n"
        let table = try CSVTable(parsing: text)
        #expect(table.header == ["key", "display"])
        #expect(table.rows == [["apple", "Apple"], ["pear", "Pear"]])
    }

    @Test("keeps empty cells and a trailing empty field")
    func emptyFields() throws {
        let table = try CSVTable(parsing: "key,display,pos\napple,,\npear,Pear,noun")
        #expect(table.rows[0] == ["apple", "", ""])
        #expect(table.rows[1] == ["pear", "Pear", "noun"])
    }

    @Test("rejects an unterminated quote")
    func unterminatedQuote() {
        #expect(throws: CSVParseError.self) {
            try CSVTable(parsing: "key,display\n\"apple,Apple\n")
        }
    }

    @Test("rejects an empty file")
    func emptyFile() {
        #expect(throws: CSVParseError.emptyFile) {
            try CSVTable(parsing: "")
        }
    }
}

@Suite("CSV import plan")
struct CSVImportPlanTests {
    @Test("merges rows sharing a key into multiple definitions")
    func mergesDefinitions() throws {
        let table = try CSVTable(
            parsing: """
            key,display,phonetic,pos,sense,examples,labels
            bank,Bank,bæŋk,noun,a financial institution,Save at the bank,finance
            bank,Bank,,noun,the side of a river,,\"nature; geography\"
            """
        )
        let mapping = CSVColumnMapping.suggested(for: table.header)
        let plan = try #require(CSVImportPlan.make(table: table, mapping: mapping))
        #expect(plan.groups.count == 1)
        let entry = try #require(plan.entries.first)
        #expect(entry.key == "bank")
        #expect(entry.display == "Bank")
        #expect(entry.phonetic == ["bæŋk"])
        #expect(entry.defs.count == 2)
        #expect(entry.defs[0].examples == ["Save at the bank"])
        #expect(entry.defs[1].labels == ["nature", "geography"])
        #expect(plan.totalRows == 2)
        #expect(plan.issues.isEmpty)
    }

    @Test("skips empty keys and reports each row")
    func skipsEmptyKeys() throws {
        let table = try CSVTable(parsing: "key,display\napple,Apple\n,No key\n\npear,Pear")
        let plan = try #require(
            CSVImportPlan.make(table: table, mapping: .suggested(for: table.header))
        )
        #expect(plan.groups.count == 2)
        // Row numbers follow the sheet: the header is row 1.
        #expect(plan.issues.map(\.row) == [3, 4])
        #expect(plan.totalRows == 4)
    }

    @Test("requires a key column")
    func requiresKey() throws {
        let table = try CSVTable(parsing: "display\nApple")
        #expect(CSVImportPlan.make(table: table, mapping: .suggested(for: table.header)) == nil)
    }

    @Test("writes one entry per key and reports failures")
    @MainActor
    func importsIntoWorkingSet() throws {
        let fake = FakeEditing(entries: [makeEntry("seed")])
        fake.failOnKeys = ["bad"]
        let model = CSVImportModel(editing: fake)
        model.load(
            url: try writeCSV(
                tableText: "key,display,pos,sense\napple,Apple,noun,a fruit\nbad,Bad,noun,refused"
            )
        )

        let plan = try #require(model.preparedPlan)
        let result = model.performImport(plan: plan)
        #expect(result.importedEntries == 1)
        #expect(result.failedKeys == ["bad"])
        #expect(result.skippedRows == 1)
        #expect(fake.working["apple"]?.display == "Apple")
        #expect(model.phase == .finished)
        #expect(model.processedRows == 2)
    }

    @Test("advances progress while importing through the task")
    @MainActor
    func startImportFinishes() async throws {
        let fake = FakeEditing(entries: [makeEntry("seed")])
        let model = CSVImportModel(editing: fake)
        model.load(url: try writeCSV(tableText: "key,display\napple,Apple\npear,Pear"))
        model.startImport()
        var attempts = 0
        while model.phase != .finished, attempts < 100 {
            try await Task.sleep(for: .milliseconds(5))
            attempts += 1
        }
        #expect(model.phase == .finished)
        #expect(model.importedEntries == 2)
    }

    private func writeCSV(tableText: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appending(path: "odc-csv-\(UUID().uuidString).csv")
        try tableText.write(to: url, atomically: true, encoding: .utf8)
        return url
    }
}
