import Foundation
import Observation

/// Editable projection of one entry; the view binds to this, the model turns
/// it back into a `DictionaryEntry` on save.
public struct EntryEditorForm: Equatable, Sendable {
    public var key: String
    public var display: String
    public var phoneticText: String
    public var definitions: [DefinitionForm]
    public var rawHTML: String

    public init(
        key: String,
        display: String,
        phoneticText: String = "",
        definitions: [DefinitionForm] = [],
        rawHTML: String = ""
    ) {
        self.key = key
        self.display = display
        self.phoneticText = phoneticText
        self.definitions = definitions
        self.rawHTML = rawHTML
    }

    public init(entry: DictionaryEntry) {
        self.init(
            key: entry.key,
            display: entry.display,
            phoneticText: entry.phonetic.joined(separator: ", "),
            definitions: entry.defs.map(DefinitionForm.init(definition:)),
            rawHTML: entry.rawHTML ?? ""
        )
    }

    /// Build the entry that `upsert` writes, preserving fields the editor
    /// does not model.
    public func makeEntry(extra: [String: JSONValue]) -> DictionaryEntry {
        let phonetic = Self.split(self.phoneticText, separators: [",", "\n"])
        let defs = definitions.compactMap { definition -> Definition? in
            let pos = definition.pos.trimmingCharacters(in: .whitespacesAndNewlines)
            let sense = definition.sense.trimmingCharacters(in: .whitespacesAndNewlines)
            let examples = Self.split(definition.examplesText, separators: ["\n"])
            let labels = Self.split(definition.labelsText, separators: [",", ";", "\n"])
            guard !pos.isEmpty || !sense.isEmpty || !examples.isEmpty || !labels.isEmpty else {
                return nil
            }
            return Definition(pos: pos, sense: sense, examples: examples, labels: labels)
        }
        let html = rawHTML.trimmingCharacters(in: .whitespacesAndNewlines)
        return DictionaryEntry(
            key: key.trimmingCharacters(in: .whitespacesAndNewlines),
            display: display.trimmingCharacters(in: .whitespacesAndNewlines),
            phonetic: phonetic,
            defs: defs,
            rawHTML: html.isEmpty ? nil : rawHTML,
            extra: extra
        )
    }

    private static func split(_ text: String, separators: [Character]) -> [String] {
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

public struct DefinitionForm: Identifiable, Equatable, Sendable {
    public let id: UUID
    public var pos: String
    public var sense: String
    public var examplesText: String
    public var labelsText: String

    public init(
        id: UUID = UUID(),
        pos: String = "",
        sense: String = "",
        examplesText: String = "",
        labelsText: String = ""
    ) {
        self.id = id
        self.pos = pos
        self.sense = sense
        self.examplesText = examplesText
        self.labelsText = labelsText
    }

    public init(definition: Definition) {
        self.init(
            pos: definition.pos,
            sense: definition.sense,
            examplesText: definition.examples.joined(separator: "\n"),
            labelsText: definition.labels.joined(separator: ", ")
        )
    }
}

/// Drives the macOS editing window: paged browsing, prefix search, the detail
/// form, field diff, status and the commit/discard lifecycle.
@MainActor
@Observable
public final class DictionaryEditorModel {
    public struct Row: Identifiable, Equatable, Sendable {
        public let key: String
        public let display: String
        public let summary: String
        public let change: ChangeKind?

        public var id: String { key }

        public init(key: String, display: String, summary: String, change: ChangeKind?) {
            self.key = key
            self.display = display
            self.summary = summary
            self.change = change
        }
    }

    public static let pageSize = 200
    public static let searchLimit = 200

    public private(set) var rows: [Row] = []
    public private(set) var hasMore = false
    public private(set) var isLoading = false
    public private(set) var status: WorkingStatus
    public private(set) var isLoaded = false
    public var selectedKey: String?
    public var searchText = ""
    public private(set) var form: EntryEditorForm?
    public private(set) var original: DictionaryEntry?
    public private(set) var diff: EntryDiff?
    public var showOnlyChanged = true
    public var validationMessage: String?
    public var alertMessage: String?
    public private(set) var noticeMessage: String?
    public private(set) var committed = false
    public var author: String

    @ObservationIgnored private let editing: ODCEditing
    @ObservationIgnored private let onCommitHandler: (() -> Void)?
    @ObservationIgnored private var loadedEntries: [DictionaryEntry] = []
    @ObservationIgnored private var deletedRows: [Row] = []
    @ObservationIgnored private var changeKinds: [String: ChangeKind] = [:]
    @ObservationIgnored private var lastLoadedKey: String?
    @ObservationIgnored private var searching = false

    public init(
        editing: ODCEditing,
        author: String = NSUserName(),
        onCommit: (() -> Void)? = nil
    ) {
        self.editing = editing
        self.author = author
        self.onCommitHandler = onCommit
        self.status = WorkingStatus(
            added: 0,
            modified: 0,
            removed: 0,
            total: 0,
            entries: 0,
            blocks: 0,
            head: nil
        )
    }

    public var hasChanges: Bool {
        status.added + status.modified + status.removed > 0
    }

    public var visibleDiffFields: [FieldDiff] {
        let fields = diff?.fields ?? []
        return showOnlyChanged ? fields.filter(\.changed) : fields
    }

    // MARK: - Loading

    public func loadIfNeeded() {
        guard !isLoaded, !isLoading else { return }
        loadFirstPage()
    }

    public func reload() {
        isLoaded = false
        loadIfNeeded()
    }

    public func loadFirstPage() {
        isLoading = true
        defer { isLoading = false }
        do {
            status = try editing.workingStatus()
            try refreshChanges()
            loadedEntries = try editing.entriesPage(after: nil, limit: Self.pageSize)
            lastLoadedKey = loadedEntries.last?.key
            hasMore = loadedEntries.count == Self.pageSize
            searching = false
            isLoaded = true
            rebuildRows()
            restoreSelection()
        } catch {
            alertMessage = error.localizedDescription
        }
    }

    public func loadMore() {
        guard !isLoading, hasMore, !searching else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let page = try editing.entriesPage(after: lastLoadedKey, limit: Self.pageSize)
            loadedEntries.append(contentsOf: page)
            lastLoadedKey = page.last?.key
            hasMore = page.count == Self.pageSize
            rebuildRows()
        } catch {
            alertMessage = error.localizedDescription
        }
    }

    public func applySearch() {
        let term = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty else {
            searching = false
            loadFirstPage()
            return
        }

        isLoading = true
        defer { isLoading = false }
        do {
            status = try editing.workingStatus()
            try refreshChanges()
            let hits = try editing.search(term, limit: Self.searchLimit)
            var entries: [DictionaryEntry] = []
            for hit in hits {
                if let entry = try? editing.entry(for: hit.key) {
                    entries.append(entry)
                }
            }
            loadedEntries = entries
            lastLoadedKey = entries.last?.key
            hasMore = false
            searching = true
            isLoaded = true
            rebuildRows()
            restoreSelection()
        } catch {
            alertMessage = error.localizedDescription
        }
    }

    public func clearSearch() {
        searchText = ""
        searching = false
        loadFirstPage()
    }

    // MARK: - Selection and editing

    public func select(_ key: String?) {
        selectedKey = key
        validationMessage = nil
        guard let key else {
            form = nil
            original = nil
            diff = nil
            return
        }

        if let entry = try? editing.entry(for: key) {
            original = entry
            form = EntryEditorForm(entry: entry)
        } else {
            // A deleted working-set key has no entry, but its diff is useful.
            original = nil
            form = nil
        }
        diff = try? editing.entryDiff(key)
    }

    public func updateForm(_ mutate: (inout EntryEditorForm) -> Void) {
        guard var form else { return }
        mutate(&form)
        self.form = form
    }

    public func addDefinition() {
        updateForm { $0.definitions.append(DefinitionForm()) }
    }

    public func removeDefinition(id: UUID) {
        updateForm { $0.definitions.removeAll { $0.id == id } }
    }

    @discardableResult
    public func save() -> Bool {
        guard let form else {
            alertMessage = "请先选择一个词条。"
            return false
        }
        guard let selectedKey else {
            alertMessage = "请先选择一个词条。"
            return false
        }

        let key = form.key.trimmingCharacters(in: .whitespacesAndNewlines)
        let display = form.display.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else {
            validationMessage = "key 不能为空。"
            return false
        }
        guard !display.isEmpty else {
            validationMessage = "display 不能为空。"
            return false
        }
        guard key == selectedKey else {
            validationMessage = "key 不可在编辑器里修改，请重新导入或新建词条。"
            return false
        }

        let entry = form.makeEntry(extra: original?.extra ?? [:])
        do {
            _ = try editing.upsert(entry)
            original = entry
            self.form = EntryEditorForm(entry: entry)
            status = try editing.workingStatus()
            try refreshChanges()
            rebuildRows()
            diff = try? editing.entryDiff(entry.key)
            validationMessage = nil
            noticeMessage = "已保存到工作区。"
            return true
        } catch {
            alertMessage = error.localizedDescription
            return false
        }
    }

    // MARK: - Commit and discard

    @discardableResult
    public func commit(message: String) -> Bool {
        let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            alertMessage = "提交信息不能为空。"
            return false
        }
        guard hasChanges else {
            alertMessage = "工作区没有改动，不需要提交。"
            return false
        }

        do {
            _ = try editing.commit(message: trimmed, author: author)
            onCommitHandler?()
            status = try editing.workingStatus()
            try refreshChanges()
            rebuildRows()
            diff = selectedKey.flatMap { try? editing.entryDiff($0) }
            noticeMessage = "已提交。"
            committed = true
            return true
        } catch {
            alertMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func discard() -> Bool {
        do {
            _ = try editing.discardWorking()
            loadedEntries = []
            lastLoadedKey = nil
            deletedRows = []
            changeKinds = [:]
            selectedKey = nil
            form = nil
            original = nil
            diff = nil
            searchText = ""
            searching = false
            noticeMessage = "已丢弃工作区改动。"
            loadFirstPage()
            return true
        } catch {
            alertMessage = error.localizedDescription
            return false
        }
    }

    public func clearAlert() {
        alertMessage = nil
    }

    public func clearValidation() {
        validationMessage = nil
    }

    // MARK: - Rows

    private func rebuildRows() {
        var entriesByKey: [String: DictionaryEntry] = [:]
        for entry in loadedEntries {
            entriesByKey[entry.key] = entry
        }

        var rows = loadedEntries.map { entry in
            Row(
                key: entry.key,
                display: entry.display,
                summary: Self.summary(for: entry),
                change: changeKinds[entry.key]
            )
        }

        for row in deletedRows where entriesByKey[row.key] == nil {
            rows.append(row)
        }

        rows.sort { $0.key < $1.key }
        self.rows = rows
    }

    private func restoreSelection() {
        guard let selectedKey else { return }
        if rows.contains(where: { $0.key == selectedKey }) {
            select(selectedKey)
        } else {
            self.selectedKey = nil
            form = nil
            original = nil
            diff = nil
        }
    }

    private func refreshChanges() throws {
        var kinds: [String: ChangeKind] = [:]
        var after: String?
        var guardCount = 0
        while guardCount < 1000 {
            guardCount += 1
            let page = try editing.workingChanges(after: after, limit: 200)
            guard !page.isEmpty else { break }
            for row in page {
                kinds[row.key] = row.kind
            }
            after = page.last?.key
            if page.count < 200 { break }
        }
        changeKinds = kinds
        deletedRows = kinds.compactMap { key, kind in
            guard kind == .deleted else { return nil }
            return Row(key: key, display: key, summary: "已删除", change: .deleted)
        }
    }

    private static func summary(for entry: DictionaryEntry) -> String {
        let parts = entry.defs.prefix(2).map { definition -> String in
            let pos = definition.pos.isEmpty ? "" : definition.pos
            let sense = definition.sense
            if pos.isEmpty { return sense }
            if sense.isEmpty { return pos }
            return "\(pos) \(sense)"
        }
        return parts.joined(separator: " · ")
    }
}
