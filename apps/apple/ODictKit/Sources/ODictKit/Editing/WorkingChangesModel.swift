import Foundation
import ODictCore

/// One row of the changes panel.
public struct WorkingChangeItem: Identifiable, Equatable, Sendable {
    public let key: String
    public let kind: ChangeKind

    public var id: String { key }

    public init(key: String, kind: ChangeKind) {
        self.key = key
        self.kind = kind
    }
}

/// A change kind plus the rows that belong to it.
public struct WorkingChangeSection: Identifiable, Equatable, Sendable {
    public let kind: ChangeKind
    public let items: [WorkingChangeItem]

    public var id: String { kind.rawValue }
    public var count: Int { items.count }

    public init(kind: ChangeKind, items: [WorkingChangeItem]) {
        self.kind = kind
        self.items = items
    }
}

/// One rendered field of an entry diff.
///
/// Values are pre-formatted so the view never has to decide what a missing
/// or empty value looks like.
public struct WorkingFieldRow: Identifiable, Equatable, Sendable {
    public let field: String
    public let before: String
    public let after: String
    public let changed: Bool

    public var id: String { field }

    public init(field: String, before: String, after: String, changed: Bool) {
        self.field = field
        self.before = before
        self.after = after
        self.changed = changed
    }
}

/// State behind the editor's changes panel.
///
/// The model talks to the repository only through `ODCEditing`, so tests can
/// drive the whole flow with an in-memory fake. Synchronous like the rest of
/// the editor: the FFI calls are local and short.
@MainActor
@Observable
public final class WorkingChangesModel {
    /// Placeholder for a field that does not exist on one side.
    public static let missingValue = "—"
    /// Placeholder for a field that exists but holds an empty string.
    public static let emptyValue = "（空）"
    /// Shown after a commit; long enough to stay unique in a history list.
    public static let shortHashLength = 7

    /// Order of the sections in the panel.
    public static let kindOrder: [ChangeKind] = [.added, .modified, .deleted]

    public private(set) var sections: [WorkingChangeSection] = []
    public private(set) var status: WorkingStatus
    public private(set) var isRefreshing = false
    public private(set) var isCommitting = false
    public private(set) var errorMessage: String?
    public private(set) var lastCommitHash: String?

    public var selectedKey: String?
    public private(set) var diff: EntryDiff?
    public private(set) var fieldRows: [WorkingFieldRow] = []

    /// Commit summary. Follows the automatic summary until the user edits it.
    public var commitMessage: String = "" {
        didSet {
            guard commitMessage != oldValue else { return }
            if commitMessage != autoSummary {
                hasCustomMessage = true
            }
        }
    }

    @ObservationIgnored private let editing: ODCEditing
    @ObservationIgnored private let author: String
    @ObservationIgnored private var hasCustomMessage = false

    public init(editing: ODCEditing, author: String) {
        self.editing = editing
        self.author = author
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

    // MARK: - Derived

    public var hasChanges: Bool {
        status.added + status.modified + status.removed > 0
    }

    public var changeCount: Int {
        status.added + status.modified + status.removed
    }

    /// One-line summary used as the default commit message.
    public var autoSummary: String {
        Self.summaryText(added: status.added, modified: status.modified, deleted: status.removed)
    }

    public var canCommit: Bool {
        hasChanges && !isCommitting
            && !commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Builds the automatic summary, e.g. `Add 3, modify 5, delete 1`.
    ///
    /// Zero counts are dropped so a single-kind change reads naturally; a
    /// completely clean working set yields an empty string.
    public static func summaryText(added: Int, modified: Int, deleted: Int) -> String {
        var parts: [String] = []
        if added > 0 { parts.append("Add \(added)") }
        if modified > 0 { parts.append("modify \(modified)") }
        if deleted > 0 { parts.append("delete \(deleted)") }
        return parts.joined(separator: ", ")
    }

    /// Truncates a commit hash for display.
    public static func shortHash(_ hash: String) -> String {
        String(hash.prefix(shortHashLength))
    }

    // MARK: - Loading

    /// Re-reads the working set from the repository and rebuilds the groups.
    public func refresh() {
        isRefreshing = true
        errorMessage = nil
        defer { isRefreshing = false }
        do {
            status = try editing.workingStatus()
            let rows = try loadAllChanges()
            sections = Self.sections(from: rows)
            if let selectedKey, !rows.contains(where: { $0.key == selectedKey }) {
                self.selectedKey = nil
                diff = nil
                fieldRows = []
            }
            applyAutoSummary()
            if let selectedKey {
                loadDiff(for: selectedKey)
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Selects a row and loads its field-level diff.
    public func select(_ key: String?) {
        guard selectedKey != key else { return }
        selectedKey = key
        if let key {
            loadDiff(for: key)
        } else {
            diff = nil
            fieldRows = []
        }
    }

    private func loadDiff(for key: String) {
        do {
            let entryDiff = try editing.entryDiff(key)
            diff = entryDiff
            fieldRows = Self.fieldRows(for: entryDiff)
        } catch {
            diff = nil
            fieldRows = []
            errorMessage = error.localizedDescription
        }
    }

    private func loadAllChanges() throws -> [ChangeRow] {
        var rows: [ChangeRow] = []
        var after: String?
        // The repository paginates by key; a generous cap keeps a broken
        // cursor from spinning forever.
        for _ in 0..<10_000 {
            let page = try editing.workingChanges(after: after, limit: 200)
            guard !page.isEmpty else { break }
            rows.append(contentsOf: page)
            after = page.last?.key
            if page.count < 200 { break }
        }
        return rows
    }

    /// Groups rows in panel order, dropping kinds with no rows.
    public static func sections(from rows: [ChangeRow]) -> [WorkingChangeSection] {
        kindOrder.compactMap { kind in
            let items = rows
                .filter { $0.kind == kind }
                .map { WorkingChangeItem(key: $0.key, kind: $0.kind) }
                .sorted { $0.key < $1.key }
            return items.isEmpty ? nil : WorkingChangeSection(kind: kind, items: items)
        }
    }

    /// Renders an entry diff for the field table.
    public static func fieldRows(for diff: EntryDiff) -> [WorkingFieldRow] {
        diff.fields.map { field in
            WorkingFieldRow(
                field: field.field,
                before: displayText(field.before),
                after: displayText(field.after),
                changed: field.changed
            )
        }
    }

    /// A missing side and an empty string read differently on purpose: one
    /// means the field was added or removed, the other means it was cleared.
    public static func displayText(_ value: JSONValue?) -> String {
        guard let value else { return missingValue }
        if case .string(let text) = value, text.isEmpty { return emptyValue }
        return value.displayText
    }

    // MARK: - Commit

    @discardableResult
    public func commit() -> Bool {
        guard !isCommitting else { return false }
        guard hasChanges else {
            errorMessage = "工作区没有改动，不需要提交。"
            return false
        }
        let message = commitMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else {
            errorMessage = "提交信息不能为空。"
            return false
        }

        isCommitting = true
        defer { isCommitting = false }
        do {
            let result = try editing.commit(message: message, author: author)
            lastCommitHash = result.hash
            hasCustomMessage = false
            selectedKey = nil
            diff = nil
            fieldRows = []
            refresh()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func discardAll() -> Bool {
        do {
            _ = try editing.discardWorking()
            selectedKey = nil
            diff = nil
            fieldRows = []
            hasCustomMessage = false
            refresh()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    private func applyAutoSummary() {
        guard !hasCustomMessage else { return }
        commitMessage = autoSummary
    }
}
