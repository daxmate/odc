import Foundation
import Testing
@testable import ODictKit

@Suite("Working changes model")
@MainActor
struct WorkingChangesModelTests {
    private func makeModel(
        entries: [DictionaryEntry] = [makeEntry("apple"), makeEntry("banana")],
        changes: [(String, ChangeKind)] = [("apple", .modified)]
    ) -> (WorkingChangesModel, FakeEditing) {
        let fake = FakeEditing(entries: entries, changes: changes)
        let model = WorkingChangesModel(editing: fake, author: "tester")
        model.refresh()
        return (model, fake)
    }

    // MARK: - Formatting

    @Test("summarizes counts in panel order")
    func summaryText() {
        #expect(
            WorkingChangesModel.summaryText(added: 3, modified: 5, deleted: 1)
                == "Add 3, modify 5, delete 1"
        )
        #expect(WorkingChangesModel.summaryText(added: 0, modified: 2, deleted: 0) == "modify 2")
        #expect(WorkingChangesModel.summaryText(added: 1, modified: 0, deleted: 4) == "Add 1, delete 4")
        #expect(WorkingChangesModel.summaryText(added: 0, modified: 0, deleted: 0).isEmpty)
    }

    @Test("shortens commit hashes")
    func shortHash() {
        #expect(WorkingChangesModel.shortHash("0123456789abcdef") == "0123456")
        #expect(WorkingChangesModel.shortHash("abc") == "abc")
    }

    @Test("groups rows by kind, sorted by key")
    func grouping() {
        let rows = [
            ChangeRow(key: "cherry", kind: .deleted),
            ChangeRow(key: "banana", kind: .modified),
            ChangeRow(key: "apple", kind: .added),
            ChangeRow(key: "ant", kind: .added),
        ]
        let sections = WorkingChangesModel.sections(from: rows)
        #expect(sections.map(\.kind) == [.added, .modified, .deleted])
        #expect(sections.map(\.count) == [2, 1, 1])
        #expect(sections[0].items.map(\.key) == ["ant", "apple"])
    }

    @Test("renders missing and empty values differently")
    func fieldPlaceholders() {
        let diff = EntryDiff(
            key: "apple",
            kind: .modified,
            fields: [
                FieldDiff(field: "display", before: .string("A"), after: .string("B"), changed: true),
                FieldDiff(field: "extra.source", before: nil, after: .string(""), changed: true),
                FieldDiff(field: "phonetic[0].text", before: .string("/x/"), after: .string("/x/"), changed: false),
            ]
        )
        let rows = WorkingChangesModel.fieldRows(for: diff)
        #expect(rows.count == 3)
        #expect(rows[0].before == "A")
        #expect(rows[0].after == "B")
        #expect(rows[1].before == WorkingChangesModel.missingValue)
        #expect(rows[1].after == WorkingChangesModel.emptyValue)
        #expect(rows[2].changed == false)
    }

    // MARK: - Loading

    @Test("loads the working set and prefills the message")
    func load() {
        let (model, _) = makeModel()
        #expect(model.hasChanges)
        #expect(model.changeCount == 1)
        #expect(model.sections.map(\.kind) == [.modified])
        #expect(model.commitMessage == "modify 1")
        #expect(model.canCommit)
    }

    @Test("keeps a custom message across refreshes")
    func customMessage() {
        let (model, _) = makeModel()
        model.commitMessage = "Fix the apple entry"
        model.refresh()
        #expect(model.commitMessage == "Fix the apple entry")
    }

    @Test("keeps following the summary until the user edits it")
    func autoMessage() {
        let (model, fake) = makeModel()
        fake.working["cherry"] = makeEntry("cherry")
        model.refresh()
        #expect(model.commitMessage == "Add 1, modify 1")
    }

    @Test("loads the diff for the selected row")
    func selection() {
        let diff = EntryDiff(
            key: "apple",
            kind: .modified,
            fields: [FieldDiff(field: "display", before: .string("Apple"), after: .string("Apple!"), changed: true)]
        )
        let (model, fake) = makeModel()
        fake.diffs["apple"] = diff
        model.select("apple")
        #expect(model.diff?.key == "apple")
        #expect(model.fieldRows.map(\.field) == ["display"])
        model.select(nil)
        #expect(model.fieldRows.isEmpty)
    }

    @Test("drops a selection that no longer has changes")
    func selectionClearedOnRefresh() {
        let (model, fake) = makeModel()
        model.select("apple")
        fake.working = fake.head
        model.refresh()
        #expect(model.selectedKey == nil)
        #expect(model.hasChanges == false)
    }

    // MARK: - Commit and discard

    @Test("commits with the prefilled message and reports the short hash")
    func commit() {
        let (model, fake) = makeModel()
        #expect(model.commit())
        #expect(fake.commitMessage == "modify 1")
        #expect(fake.commitAuthor == "tester")
        #expect(model.lastCommitHash == fake.commitHash)
        #expect(model.hasChanges == false)
        #expect(model.sections.isEmpty)
    }

    @Test("refuses an empty commit message")
    func commitWithoutMessage() {
        let (model, fake) = makeModel()
        model.commitMessage = "   "
        #expect(model.commit() == false)
        #expect(fake.commitMessage == nil)
        #expect(model.errorMessage == "提交信息不能为空。")
    }

    @Test("refuses to commit a clean working set")
    func commitWithoutChanges() {
        let (model, _) = makeModel(changes: [])
        #expect(model.hasChanges == false)
        #expect(model.commit() == false)
        #expect(model.errorMessage == "工作区没有改动，不需要提交。")
    }

    @Test("discards every local change")
    func discard() {
        let (model, fake) = makeModel(changes: [("apple", .modified), ("banana", .deleted)])
        #expect(model.hasChanges)
        #expect(model.discardAll())
        #expect(fake.discardCount == 1)
        #expect(model.hasChanges == false)
        #expect(model.sections.isEmpty)
    }

    @Test("surfaces repository failures")
    func failure() {
        let (model, fake) = makeModel()
        fake.diffFailures.insert("apple")
        model.select("apple")
        #expect(model.errorMessage != nil)
        #expect(model.fieldRows.isEmpty)
    }
}
