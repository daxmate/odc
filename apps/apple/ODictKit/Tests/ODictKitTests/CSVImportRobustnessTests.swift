import Foundation
import Testing
@testable import ODictKit

@Suite("CSV text decoding")
struct CSVTextDecodingTests {
    private func csvText() -> String {
        "key,display,sense\napple,苹果,a fruit\npear,梨,a fruit"
    }

    @Test("strips a UTF-8 byte order mark and labels the encoding")
    func utf8BOM() throws {
        var data = Data([0xEF, 0xBB, 0xBF])
        data.append(Data(csvText().utf8))
        let decoded = try CSVTextDecoder.decode(data)
        #expect(decoded.encodingLabel == "UTF-8 with BOM")
        #expect(!decoded.text.hasPrefix("\u{FEFF}"))
        #expect(decoded.text.hasPrefix("key,display"))
    }

    @Test("reads plain UTF-8")
    func utf8() throws {
        let decoded = try CSVTextDecoder.decode(Data(csvText().utf8))
        #expect(decoded.encodingLabel == "UTF-8")
        #expect(decoded.text == csvText())
    }

    @Test("reads GB18030, the encoding Excel writes on Chinese systems")
    func gb18030() throws {
        let data = try #require(csvText().data(using: CSVTextDecoder.gb18030))
        let decoded = try CSVTextDecoder.decode(data)
        #expect(decoded.encodingLabel == "GB18030")
        #expect(decoded.text == csvText())
    }

    @Test("reads UTF-16 with a byte order mark")
    func utf16() throws {
        var data = Data([0xFF, 0xFE])
        data.append(try #require(csvText().data(using: .utf16LittleEndian)))
        let decoded = try CSVTextDecoder.decode(data)
        #expect(decoded.encodingLabel == "UTF-16 LE")
        #expect(decoded.text == csvText())
    }

    @Test("rejects bytes no supported encoding accepts")
    func invalidBytes() {
        let data = Data([0x80, 0x81, 0xFF, 0xFE, 0x00])
        #expect(throws: CSVTextDecodingError.unrecognizedEncoding) {
            _ = try CSVTextDecoder.decode(data)
        }
        #expect(
            CSVTextDecodingError.unrecognizedEncoding.errorDescription
                == "无法识别文件编码。请在 Excel/Numbers 中另存为 UTF-8 CSV 后重试。"
        )
    }

    @Test("the import model reports the detected encoding")
    @MainActor
    func modelShowsEncoding() throws {
        let fake = FakeEditing(entries: [])
        let model = CSVImportModel(editing: fake)
        let data = try #require(csvText().data(using: CSVTextDecoder.gb18030))
        let url = FileManager.default.temporaryDirectory
            .appending(path: "odc-csv-gb-\(UUID().uuidString).csv")
        try data.write(to: url)
        model.load(url: url)
        #expect(model.encodingLabel == "GB18030")
        #expect(model.table?.rows.count == 2)
    }

    @Test("an undecodable file surfaces the actionable message")
    @MainActor
    func modelReportsUndecodableFile() throws {
        let fake = FakeEditing(entries: [])
        let model = CSVImportModel(editing: fake)
        let url = FileManager.default.temporaryDirectory
            .appending(path: "odc-csv-bad-\(UUID().uuidString).csv")
        try Data([0x80, 0x81, 0xFF, 0xFE, 0x00]).write(to: url)
        model.load(url: url)
        #expect(model.table == nil)
        #expect(model.alertMessage?.contains("无法识别文件编码") == true)
    }
}

@Suite("CSV duplicate policy")
struct CSVDuplicatePolicyTests {
    private func table(_ text: String) throws -> CSVTable {
        try CSVTable(parsing: text)
    }

    @Test("merge folds rows that share a key")
    func mergeWithinCSV() throws {
        let table = try table("key,display,sense\napple,Apple,a fruit\napple,,a company")
        let plan = try #require(CSVImportPlan.make(table: table, mapping: .suggested(for: table.header)))
        #expect(plan.groups.count == 1)
        #expect(plan.groups[0].entry.defs.count == 2)
        #expect(plan.groups[0].sourceRows == [2, 3])
        #expect(plan.issues.isEmpty)
    }

    @Test("skip keeps the first row and reports the duplicate")
    func skipWithinCSV() throws {
        let table = try table("key,display,sense\napple,Apple,a fruit\napple,,a company")
        let plan = try #require(
            CSVImportPlan.make(table: table, mapping: .suggested(for: table.header), duplicatePolicy: .skip)
        )
        #expect(plan.groups.count == 1)
        #expect(plan.groups[0].entry.defs.count == 1)
        #expect(plan.groups[0].entry.display == "Apple")
        #expect(plan.issues.count == 1)
        #expect(plan.issues[0].row == 3)
        #expect(plan.issues[0].column == "key")
        #expect(plan.issues[0].reason.contains("重复 key"))
    }

    @Test("overwrite keeps the last row")
    func overwriteWithinCSV() throws {
        let table = try table("key,display,sense\napple,Apple,a fruit\napple,Apple II,a company")
        let plan = try #require(
            CSVImportPlan.make(table: table, mapping: .suggested(for: table.header), duplicatePolicy: .overwrite)
        )
        #expect(plan.groups.count == 1)
        #expect(plan.groups[0].entry.display == "Apple II")
        #expect(plan.groups[0].entry.defs.map(\.sense) == ["a company"])
        #expect(plan.groups[0].sourceRows == [3])
    }

    @Test("library conflicts are detected before importing")
    @MainActor
    func detectsLibraryConflicts() throws {
        let fake = FakeEditing(entries: [supportEntry("apple")])
        let table = try table("key,display\napple,Apple\npear,Pear")
        let summary = CSVDuplicateDetector.summary(
            table: table,
            mapping: .suggested(for: table.header),
            editing: fake
        )
        #expect(summary.libraryConflicts == ["apple"])
        #expect(summary.internalDuplicates.isEmpty)
        #expect(summary.affectedKeyCount == 1)
    }

    @Test("skip leaves the stored entry untouched")
    @MainActor
    func librarySkip() throws {
        let original = supportEntry("apple", display: "Stored")
        let fake = FakeEditing(entries: [original])
        let model = CSVImportModel(editing: fake)
        model.load(url: try writeCSV("key,display,sense\napple,Incoming,a fruit"))
        let plan = try #require(model.preparedPlan)
        let report = model.performImport(plan: plan, policy: .skip)
        #expect(report.importedEntries == 0)
        #expect(report.overwrittenEntries == 0)
        #expect(report.mergedEntries == 0)
        #expect(report.skippedRows == 1)
        #expect(fake.working["apple"]?.display == "Stored")
        #expect(model.rowIssues.contains { $0.reason.contains("库内已存在") })
    }

    @Test("overwrite replaces the stored entry")
    @MainActor
    func libraryOverwrite() throws {
        let fake = FakeEditing(entries: [supportEntry("apple", display: "Stored")])
        let model = CSVImportModel(editing: fake)
        model.load(url: try writeCSV("key,display,sense\napple,Incoming,a fruit"))
        let plan = try #require(model.preparedPlan)
        let report = model.performImport(plan: plan, policy: .overwrite)
        #expect(report.overwrittenEntries == 1)
        #expect(report.importedEntries == 0)
        #expect(fake.working["apple"]?.display == "Incoming")
    }

    @Test("merge fills empty CSV fields from the library")
    @MainActor
    func libraryMerge() throws {
        let stored = supportEntry("apple", display: "Stored", sense: "old sense")
        let fake = FakeEditing(entries: [stored])
        let model = CSVImportModel(editing: fake)
        model.load(url: try writeCSV("key,display,sense\napple,,\n"))
        let plan = try #require(model.preparedPlan)
        let report = model.performImport(plan: plan, policy: .merge)
        #expect(report.mergedEntries == 1)
        let merged = try #require(fake.working["apple"])
        #expect(merged.display == "Stored")
        #expect(merged.defs.map(\.sense) == ["old sense"])
    }

    @Test("new keys are counted as imported")
    @MainActor
    func importedCount() throws {
        let fake = FakeEditing(entries: [supportEntry("seed")])
        let model = CSVImportModel(editing: fake)
        model.load(url: try writeCSV("key,display\napple,Apple\npear,Pear"))
        let plan = try #require(model.preparedPlan)
        let report = model.performImport(plan: plan)
        #expect(report.importedEntries == 2)
        #expect(report.skippedRows == 0)
    }
}

@Suite("CSV issues and mapping templates")
struct CSVIssueAndTemplateTests {
    @Test("issues carry the sheet row and the column name")
    func issueLocation() throws {
        let table = try CSVTable(parsing: "key,display\n,,orphan\napple,Apple")
        let plan = try #require(CSVImportPlan.make(table: table, mapping: .suggested(for: table.header)))
        #expect(plan.issues.map(\.row) == [2])
        #expect(plan.issues[0].column == "key")
        #expect(plan.issues[0].displayText == "第 2 行 · 列「key」：key 为空")
    }

    @Test("oversized cells are rejected with the column name")
    func oversizedField() throws {
        let long = String(repeating: "a", count: CSVImportLimits.maxFieldLength + 1)
        let table = try CSVTable(parsing: "key,display\napple,\(long)")
        let plan = try #require(CSVImportPlan.make(table: table, mapping: .suggested(for: table.header)))
        #expect(plan.groups.isEmpty)
        #expect(plan.issues[0].row == 2)
        #expect(plan.issues[0].column == "display")
        #expect(plan.issues[0].reason.contains("字段超过"))
    }

    @Test("templates are stored per library and match by header")
    @MainActor
    func templateStoreRoundTrip() throws {
        let suite = "odc-templates-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = CSVColumnTemplateStore(defaults: defaults)

        let mapping = CSVColumnMapping(key: 0, display: 1, phonetic: nil, pos: nil, sense: 2, examples: nil, labels: nil)
        store.save(
            CSVColumnMappingTemplate(name: "我的词表", header: ["Key", "Display", "Sense"], mapping: mapping),
            for: "library-a"
        )

        #expect(store.templates(for: "library-a").count == 1)
        #expect(store.templates(for: "library-b").isEmpty)
        #expect(
            store.match(header: [" key ", "display", "SENSE"], for: "library-a")?.name == "我的词表"
        )
        #expect(store.match(header: ["key", "display"], for: "library-a") == nil)

        store.delete(named: "我的词表", for: "library-a")
        #expect(store.templates(for: "library-a").isEmpty)
    }

    @Test("loading a matching file applies the saved mapping")
    @MainActor
    func templateAutoApply() throws {
        let suite = "odc-templates-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = CSVColumnTemplateStore(defaults: defaults)
        let custom = CSVColumnMapping(key: 1, display: 0, phonetic: nil, pos: nil, sense: 2, examples: nil, labels: nil)
        store.save(
            CSVColumnMappingTemplate(name: "交换列", header: ["display", "key", "sense"], mapping: custom),
            for: "lib"
        )

        let fake = FakeEditing(entries: [])
        let model = CSVImportModel(editing: fake, libraryKey: "lib", templateStore: store)
        model.load(url: try writeCSV("display,key,sense\nApple,apple,a fruit"))
        #expect(model.matchedTemplateName == "交换列")
        #expect(model.mapping.key == 1)
        #expect(model.mapping.display == 0)
        #expect(model.preparedPlan?.entries.first?.key == "apple")
    }

    @Test("the model saves the current mapping as a template")
    @MainActor
    func modelSavesTemplate() throws {
        let suite = "odc-templates-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = CSVColumnTemplateStore(defaults: defaults)
        let model = CSVImportModel(editing: FakeEditing(entries: []), libraryKey: "lib", templateStore: store)
        model.load(url: try writeCSV("key,display\napple,Apple"))

        #expect(model.saveCurrentMappingAsTemplate(named: "  基础  "))
        #expect(model.templates.map(\.name) == ["基础"])
        #expect(model.matchedTemplateName == "基础")
        model.deleteTemplate(named: "基础")
        #expect(model.templates.isEmpty)
        #expect(model.matchedTemplateName == nil)
    }
}

@Suite("CSV duplicate policy memory")
struct CSVDuplicatePolicyMemoryTests {
    @Test("remembering a choice skips the prompt next time")
    @MainActor
    func remembersChoice() async throws {
        let suite = "odc-policy-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let policyStore = CSVDuplicatePolicyStore(defaults: defaults)

        let fake = FakeEditing(entries: [supportEntry("apple", display: "Stored")])
        let first = CSVImportModel(editing: fake, libraryKey: "lib", policyStore: policyStore)
        first.load(url: try writeCSV("key,display\napple,Incoming"))
        first.startImport()
        #expect(first.isPolicyPromptPresented)
        #expect(first.duplicateSummary?.libraryConflicts == ["apple"])

        first.pendingPolicy = .skip
        first.rememberPolicy = true
        first.confirmPolicy()
        #expect(!first.isPolicyPromptPresented)
        #expect(policyStore.policy(for: "lib") == .skip)
        try await waitForFinish(first)

        #expect(first.report?.skippedRows == 1)
        #expect(fake.working["apple"]?.display == "Stored")
    }

    @Test("no prompt when the file has no duplicates")
    @MainActor
    func noPromptWithoutDuplicates() async throws {
        let suite = "odc-policy-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = CSVImportModel(
            editing: FakeEditing(entries: []),
            libraryKey: "lib",
            policyStore: CSVDuplicatePolicyStore(defaults: defaults)
        )
        model.load(url: try writeCSV("key,display\napple,Apple"))
        model.startImport()
        #expect(!model.isPolicyPromptPresented)
        try await waitForFinish(model)
        #expect(model.report?.importedEntries == 1)
    }
}

private func supportEntry(
    _ key: String,
    display: String? = nil,
    sense: String = "sense"
) -> DictionaryEntry {
    DictionaryEntry(
        key: key,
        display: display ?? key.capitalized,
        phonetic: [],
        defs: [Definition(pos: "noun", sense: sense)],
        rawHTML: nil,
        extra: [:]
    )
}

private func writeCSV(_ text: String) throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appending(path: "odc-robust-\(UUID().uuidString).csv")
    try text.write(to: url, atomically: true, encoding: .utf8)
    return url
}

@MainActor
private func waitForFinish(_ model: CSVImportModel) async throws {
    var attempts = 0
    while model.phase != .finished, attempts < 200 {
        try await Task.sleep(for: .milliseconds(5))
        attempts += 1
    }
}
