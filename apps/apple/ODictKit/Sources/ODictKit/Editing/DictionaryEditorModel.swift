import Foundation
import ODictCore
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
    /// Whether another process moved HEAD while this session has local work.
    public private(set) var hasPendingExternalChange = false

    /// Diagnostics from the last HTML check of the selected entry.
    public private(set) var htmlDiagnostics: [HtmlDiagnostic] = []
    /// Whether the last check ran (so the empty state is not a guess).
    public private(set) var htmlChecked = false
    /// Short status line: "没有发现问题" / "已修复" / "这个词条没有 HTML 内容".
    public private(set) var htmlCheckMessage: String?
    public private(set) var committed = false
    public var author: String

    /// Whether the session holds a step to undo / redo.
    public private(set) var canUndo = false
    public private(set) var canRedo = false

    @ObservationIgnored private let editing: ODCEditing
    @ObservationIgnored private let onCommitHandler: (() -> Void)?
    @ObservationIgnored private var loadedEntries: [DictionaryEntry] = []
    @ObservationIgnored private var deletedRows: [Row] = []
    @ObservationIgnored private var externalDetector = ExternalChangeDetector()
    @ObservationIgnored private var changeKinds: [String: ChangeKind] = [:]
    @ObservationIgnored private var lastLoadedKey: String?
    @ObservationIgnored private var searching = false

    /// Session-only undo/redo stacks; they die with the editor window.
    @ObservationIgnored private var undoStack: [EditorSnapshot] = []
    @ObservationIgnored private var redoStack: [EditorSnapshot] = []
    @ObservationIgnored private var lastCoalescedField: String?
    @ObservationIgnored private var lastCoalescedKey: String?
    @ObservationIgnored private var lastCoalescedAt: Date?
    @ObservationIgnored private let now: () -> Date
    @ObservationIgnored private let htmlProcessor: ODCHTMLProcessing

    public init(
        editing: ODCEditing,
        author: String = NSUserName(),
        onCommit: (() -> Void)? = nil,
        htmlProcessor: ODCHTMLProcessing = ODCHTMLProcessor(),
        now: @escaping () -> Date = { Date() }
    ) {
        self.editing = editing
        self.author = author
        self.onCommitHandler = onCommit
        self.htmlProcessor = htmlProcessor
        self.now = now
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

    /// Unsaved work: a dirty working set or a form that would write a
    /// different entry than the one it was loaded from.
    public var isDirty: Bool {
        if hasChanges { return true }
        guard let form else { return false }
        guard let original else { return true }
        return form.makeEntry(extra: original.extra) != original
    }

    /// Entries created in this session may still be renamed on save.
    public var canRenameSelectedEntry: Bool {
        guard let selectedKey else { return false }
        return changeKinds[selectedKey] == .added
    }

    /// Importer bound to the same handle; opens with the current status.
    public func makeCSVImportModel(libraryKey: String = "") -> CSVImportModel {
        CSVImportModel(editing: editing, libraryKey: libraryKey)
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
        clearUndoHistory()
        resetHTMLCheck()
        if searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            loadFirstPage()
        } else {
            applySearch()
        }
    }

    public func loadFirstPage() {
        isLoading = true
        defer { isLoading = false }
        do {
            status = try editing.workingStatus()
            if !isLoaded {
                // First load (or an explicit reload) defines the HEAD this
                // session compares against later.
                externalDetector.adopt(head: status.head)
            }
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
        resetHTMLCheck()
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

    /// Apply a form edit. Passing `field` lets consecutive keystrokes on the
    /// same field collapse into a single undo step.
    public func updateForm(
        _ field: String? = nil,
        _ mutate: (inout EntryEditorForm) -> Void
    ) {
        guard form != nil else { return }
        recordStep(touching: touchedKeys(), coalescing: field)
        mutateForm(mutate)
        validationMessage = nil
    }

    public func addDefinition() {
        guard form != nil else { return }
        recordStep(touching: touchedKeys())
        mutateForm { $0.definitions.append(DefinitionForm()) }
        validationMessage = nil
    }

    public func removeDefinition(id: UUID) {
        guard let form, form.definitions.contains(where: { $0.id == id }) else { return }
        recordStep(touching: touchedKeys())
        mutateForm { $0.definitions.removeAll { $0.id == id } }
        validationMessage = nil
    }

    // MARK: - Creating and deleting entries

    /// Insert an empty entry into the working set and select it.
    ///
    /// The key stays editable until the entry is saved. Without an explicit
    /// key a free `untitled` placeholder is used so the row shows up in the
    /// list immediately.
    @discardableResult
    public func createEntry(key suggestedKey: String? = nil) -> Bool {
        let key = (suggestedKey ?? nextDraftKey())
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else {
            validationMessage = "key 不能为空。"
            return false
        }
        guard !keyExists(key) else {
            validationMessage = "key「\(key)」已存在，请换一个。"
            return false
        }

        recordStep(touching: [key])
        do {
            _ = try editing.upsert(DictionaryEntry(key: key, display: ""))
            status = try editing.workingStatus()
            try refreshChanges()
            let stored = (try? editing.entry(for: key))
                ?? DictionaryEntry(key: key, display: "")
            if !loadedEntries.contains(where: { $0.key == stored.key }) {
                loadedEntries.append(stored)
                loadedEntries.sort { $0.key < $1.key }
            }
            lastLoadedKey = loadedEntries.last?.key
            rebuildRows()
            selectedKey = stored.key
            original = stored
            form = EntryEditorForm(
                key: stored.key,
                display: stored.display,
                definitions: [DefinitionForm()]
            )
            diff = try? editing.entryDiff(stored.key)
            validationMessage = nil
            noticeMessage = "已新建词条。"
            onCommitHandler?()
            return true
        } catch {
            alertMessage = error.localizedDescription
            return false
        }
    }

    /// Delete the selected entry from the working set.
    ///
    /// Entries that exist in HEAD are staged as deletions; entries created in
    /// this session simply disappear. The confirmation dialog lives in the
    /// view, not here.
    @discardableResult
    public func deleteEntry() -> Bool {
        guard let key = selectedKey else { return false }
        let index = rows.firstIndex { $0.key == key }

        recordStep(touching: [key])
        do {
            _ = try editing.deleteEntry(key)
            status = try editing.workingStatus()
            try refreshChanges()
            loadedEntries.removeAll { $0.key == key }
            lastLoadedKey = loadedEntries.last?.key
            rebuildRows()

            let remaining = rows.filter { $0.key != key }
            let nextKey: String?
            if remaining.isEmpty {
                nextKey = nil
            } else if let index {
                nextKey = remaining[min(index, remaining.count - 1)].key
            } else {
                nextKey = remaining.first?.key
            }
            select(nextKey)
            validationMessage = nil
            noticeMessage = "已删除词条。"
            onCommitHandler?()
            return true
        } catch {
            alertMessage = error.localizedDescription
            return false
        }
    }

    // MARK: - HTML check and repair

    /// Check the selected entry's `raw_html` without changing it.
    public func checkSelectedEntryHTML() {
        guard let rawHTML = form?.rawHTML else { return }
        htmlDiagnostics = []
        htmlChecked = false
        guard !rawHTML.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            htmlChecked = true
            htmlCheckMessage = "这个词条没有 HTML 内容。"
            return
        }
        do {
            let result = try htmlProcessor.processHTML(rawHTML, mode: .check)
            htmlDiagnostics = result.diagnostics
            htmlChecked = true
            htmlCheckMessage = result.diagnostics.isEmpty ? "没有发现问题。" : nil
        } catch {
            htmlCheckMessage = nil
            alertMessage = error.localizedDescription
        }
    }

    /// Repair the selected entry's `raw_html` through `format`.
    ///
    /// The result goes through `updateForm`, so it is one undo step and
    /// nothing reaches the working set until the user saves.
    @discardableResult
    public func applyHTMLFix() -> Bool {
        guard let rawHTML = form?.rawHTML else { return false }
        do {
            let result = try htmlProcessor.processHTML(rawHTML, mode: .format)
            if result.changed {
                updateForm { $0.rawHTML = result.html }
            }
            htmlDiagnostics = []
            htmlChecked = true
            htmlCheckMessage = result.changed
                ? "已修复 HTML，保存到工作区后生效。"
                : "没有需要修复的问题。"
            return true
        } catch {
            htmlCheckMessage = nil
            alertMessage = error.localizedDescription
            return false
        }
    }

    /// Drop stale diagnostics when the form is replaced.
    private func resetHTMLCheck() {
        htmlDiagnostics = []
        htmlChecked = false
        htmlCheckMessage = nil
    }

    // MARK: - Undo and redo

    /// Undo the last editing step of this session.
    @discardableResult
    public func undo() -> Bool {
        guard let snapshot = undoStack.last else { return false }
        let current = makeSnapshot(touching: Set(snapshot.backend.keys))
        guard restore(snapshot) else { return false }
        undoStack.removeLast()
        redoStack.append(current)
        resetCoalescing()
        refreshUndoState()
        return true
    }

    /// Re-apply the step that `undo()` reverted.
    @discardableResult
    public func redo() -> Bool {
        guard let snapshot = redoStack.last else { return false }
        let current = makeSnapshot(touching: Set(snapshot.backend.keys))
        guard restore(snapshot) else { return false }
        redoStack.removeLast()
        pushUndo(current)
        resetCoalescing()
        refreshUndoState()
        return true
    }

    // MARK: - Undo support

    /// Upper bound on session undo steps.
    public static let undoLimit = 100

    /// Edits to the same field inside this window merge into one step.
    public static let coalescingWindow: TimeInterval = 0.75

    struct EditorSnapshot {
        enum WorkingState: Equatable {
            case present(DictionaryEntry)
            case absent
        }

        var loadedEntries: [DictionaryEntry]
        var lastLoadedKey: String?
        var hasMore: Bool
        var searching: Bool
        var searchText: String
        var selectedKey: String?
        var form: EntryEditorForm?
        var original: DictionaryEntry?
        var backend: [String: WorkingState]
    }

    private func touchedKeys() -> Set<String> {
        guard let selectedKey else { return [] }
        return [selectedKey]
    }

    private func makeSnapshot(touching keys: Set<String>) -> EditorSnapshot {
        var backend: [String: EditorSnapshot.WorkingState] = [:]
        for key in keys {
            if let entry = try? editing.entry(for: key) {
                backend[key] = .present(entry)
            } else {
                backend[key] = .absent
            }
        }
        return EditorSnapshot(
            loadedEntries: loadedEntries,
            lastLoadedKey: lastLoadedKey,
            hasMore: hasMore,
            searching: searching,
            searchText: searchText,
            selectedKey: selectedKey,
            form: form,
            original: original,
            backend: backend
        )
    }

    /// Record the state before a step. `field` merges rapid edits of one
    /// field; structural operations pass `nil` and always start a new step.
    private func recordStep(
        touching keys: Set<String>,
        coalescing field: String? = nil
    ) {
        if let field, let lastCoalescedAt, !undoStack.isEmpty,
            field == lastCoalescedField,
            selectedKey == lastCoalescedKey,
            now().timeIntervalSince(lastCoalescedAt) <= Self.coalescingWindow {
            self.lastCoalescedAt = now()
            return
        }

        pushUndo(makeSnapshot(touching: keys))
        redoStack.removeAll()
        lastCoalescedField = field
        lastCoalescedKey = field == nil ? nil : selectedKey
        lastCoalescedAt = field == nil ? nil : now()
        refreshUndoState()
    }

    private func pushUndo(_ snapshot: EditorSnapshot) {
        undoStack.append(snapshot)
        if undoStack.count > Self.undoLimit {
            undoStack.removeFirst(undoStack.count - Self.undoLimit)
        }
    }

    private func restore(_ snapshot: EditorSnapshot) -> Bool {
        var backendChanged = false
        do {
            for (key, state) in snapshot.backend {
                switch state {
                case .present(let entry):
                    if (try? editing.entry(for: key)) != entry {
                        _ = try editing.upsert(entry)
                        backendChanged = true
                    }
                case .absent:
                    if (try? editing.entry(for: key)) != nil {
                        _ = try editing.deleteEntry(key)
                        backendChanged = true
                    }
                }
            }
            status = try editing.workingStatus()
            try refreshChanges()
        } catch {
            alertMessage = error.localizedDescription
            return false
        }

        loadedEntries = snapshot.loadedEntries
        lastLoadedKey = snapshot.lastLoadedKey
        hasMore = snapshot.hasMore
        searching = snapshot.searching
        searchText = snapshot.searchText
        selectedKey = snapshot.selectedKey
        form = snapshot.form
        original = snapshot.original
        diff = snapshot.selectedKey.flatMap { try? editing.entryDiff($0) }
        validationMessage = nil
        resetHTMLCheck()
        rebuildRows()
        if backendChanged {
            onCommitHandler?()
        }
        return true
    }

    private func mutateForm(_ mutate: (inout EntryEditorForm) -> Void) {
        guard var form else { return }
        mutate(&form)
        self.form = form
    }

    private func resetCoalescing() {
        lastCoalescedField = nil
        lastCoalescedKey = nil
        lastCoalescedAt = nil
    }

    private func refreshUndoState() {
        canUndo = !undoStack.isEmpty
        canRedo = !redoStack.isEmpty
    }

    private func clearUndoHistory() {
        undoStack.removeAll()
        redoStack.removeAll()
        resetCoalescing()
        refreshUndoState()
    }

    /// Mirrors the storage normalisation (`crates/odc-core`), so "Apple" and
    /// "apple" count as the same headword.
    static func normalizedKey(_ key: String) -> String {
        let folded = key.precomposedStringWithCompatibilityMapping.lowercased()
        return folded
            .replacingOccurrences(of: "ß", with: "ss")
            .replacingOccurrences(of: "ẞ", with: "ss")
            .replacingOccurrences(of: "ς", with: "σ")
    }

    /// Key uniqueness is enforced by storage (normalised upsert replaces an
    /// existing row), so the editor checks before writing. `excludingKey` lets
    /// a new entry keep its own key while it is being renamed.
    private func keyExists(_ key: String, excludingKey: String? = nil) -> Bool {
        let normalized = Self.normalizedKey(key)
        if let excludingKey, normalized == Self.normalizedKey(excludingKey) {
            return false
        }
        if loadedEntries.contains(where: { Self.normalizedKey($0.key) == normalized }) {
            return true
        }
        if rows.contains(where: { Self.normalizedKey($0.key) == normalized }) {
            return true
        }
        // Entries on pages that were never loaded still live in storage, and
        // `upsert` replaces a normalised key without asking. The lookup goes
        // through `entry(for:)` with the same normalisation the repository
        // applies, so an unseen duplicate cannot be overwritten silently.
        do {
            _ = try editing.entry(for: normalized)
            return true
        } catch let error as OdcError where error.isMissingEntry {
            return false
        } catch {
            // An unreadable repository cannot prove the key is free. Refuse
            // the write instead of guessing, and surface the real error.
            alertMessage = "无法校验 key「\(key)」是否已存在：\(error.localizedDescription)"
            return true
        }
    }

    /// Placeholder keys go through the same uniqueness check, so `untitled`
    /// never lands on an entry hiding in an unloaded page.
    private func nextDraftKey() -> String {
        var candidate = "untitled"
        var index = 1
        while keyExists(candidate) {
            index += 1
            candidate = "untitled-\(index)"
        }
        return candidate
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
        let renaming = key != selectedKey
        if renaming {
            guard changeKinds[selectedKey] == .added else {
                validationMessage = "key 不可在编辑器里修改，请重新导入或新建词条。"
                return false
            }
            guard !keyExists(key, excludingKey: selectedKey) else {
                validationMessage = "key「\(key)」已存在，请换一个。"
                return false
            }
        }

        let entry = form.makeEntry(extra: original?.extra ?? [:])
        recordStep(touching: [selectedKey, key])
        do {
            if renaming {
                _ = try editing.deleteEntry(selectedKey)
                loadedEntries.removeAll { $0.key == selectedKey }
            }
            _ = try editing.upsert(entry)
            // Storage normalises keys; show what was actually written.
            let stored = (try? editing.entry(for: key)) ?? entry
            original = stored
            self.form = EntryEditorForm(entry: stored)
            self.selectedKey = stored.key
            if !loadedEntries.contains(where: { $0.key == stored.key }) {
                loadedEntries.append(stored)
                loadedEntries.sort { $0.key < $1.key }
            }
            lastLoadedKey = loadedEntries.last?.key
            status = try editing.workingStatus()
            try refreshChanges()
            rebuildRows()
            diff = try? editing.entryDiff(stored.key)
            validationMessage = nil
            noticeMessage = "已保存到工作区。"
            // The reader shows working-set edits, so a save must refresh it
            // even though nothing is committed yet.
            onCommitHandler?()
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
            externalDetector.adopt(head: status.head)
            noticeMessage = "已提交。"
            committed = true
            clearUndoHistory()
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
            clearUndoHistory()
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

    // MARK: - Changes panel and external updates

    /// Builds the changes panel model for this editor session.
    ///
    /// The panel reads the repository directly so it can show every change,
    /// not just the page the entry list happens to hold.
    public func makeWorkingChangesModel() -> WorkingChangesModel {
        WorkingChangesModel(editing: editing, author: author)
    }

    /// Records a commit made from the changes panel: reload the list and show
    /// the short hash in the status bar.
    public func didCommit(hash: String) {
        noticeMessage = "已提交 \(WorkingChangesModel.shortHash(hash))"
        committed = true
        externalDetector.adopt(head: hash)
        reload()
    }

    /// Re-reads HEAD and decides whether another process moved the repository
    /// under this session.
    ///
    /// A clean working set reloads silently; a dirty one only raises
    /// `hasPendingExternalChange` so the user chooses what happens to their
    /// work. Nothing is ever overwritten without that choice.
    public func checkForExternalChanges() {
        guard isLoaded else { return }
        do {
            let current = try editing.workingStatus()
            let hasLocal = current.added + current.modified + current.removed > 0
            status = current
            switch externalDetector.evaluate(head: current.head, hasLocalChanges: hasLocal) {
            case .inSync:
                break
            case .reloaded:
                reload()
                noticeMessage = "库已被其他进程更新，已重新加载。"
            case .conflict:
                hasPendingExternalChange = true
            }
        } catch {
            alertMessage = error.localizedDescription
        }
    }

    /// The conflict prompt was dismissed without a decision; the next
    /// activation asks again.
    public func pendingExternalChangeDismissed() {
        hasPendingExternalChange = false
    }

    /// Keeps this session's uncommitted work and stops prompting about the
    /// foreign commit.
    public func keepLocalChanges() {
        hasPendingExternalChange = false
        externalDetector.adopt(head: status.head)
        noticeMessage = "已保留本地更改；之后的提交会写在新版本之上。"
    }

    /// Throws the local work away and adopts what the other process wrote.
    public func discardLocalChangesAfterExternalChange() {
        hasPendingExternalChange = false
        if discard() {
            externalDetector.adopt(head: status.head)
            noticeMessage = "已放弃本地更改并载入最新版本。"
        }
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

        let term = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        for row in deletedRows where entriesByKey[row.key] == nil {
            if searching && !row.key.hasPrefix(term) {
                continue
            }
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
