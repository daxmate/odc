import Foundation
import Testing
@testable import ODictKit

@Suite("Dictionary editor model")
@MainActor
struct DictionaryEditorModelTests {
    private func makeModel(
        entries: [DictionaryEntry] = [makeEntry("apple"), makeEntry("banana"), makeEntry("cherry")],
        changes: [(String, ChangeKind)] = [],
        onCommit: (() -> Void)? = nil,
        now: @escaping () -> Date = { Date() }
    ) -> (DictionaryEditorModel, FakeEditing) {
        let fake = FakeEditing(entries: entries, changes: changes)
        let model = DictionaryEditorModel(
            editing: fake,
            author: "tester",
            onCommit: onCommit,
            now: now
        )
        return (model, fake)
    }

    @Test("loads the first page with status and change badges")
    func loadsFirstPage() {
        let (model, _) = makeModel(changes: [("banana", .modified), ("cherry", .deleted)])
        model.loadIfNeeded()

        #expect(model.rows.map(\.key) == ["apple", "banana", "cherry"])
        #expect(model.rows[1].change == .modified)
        #expect(model.rows[2].change == .deleted)
        #expect(model.rows[2].summary == "已删除")
        #expect(model.status.modified == 1)
        #expect(model.status.removed == 1)
        #expect(model.hasChanges)
        #expect(model.hasMore == false)
    }

    @Test("walks pages with load more")
    func pagination() {
        let entries = (0..<201).map { makeEntry(String(format: "word%03d", $0)) }
        let (model, _) = makeModel(entries: entries)
        model.loadIfNeeded()
        #expect(model.rows.count == 200)
        #expect(model.hasMore)

        model.loadMore()
        #expect(model.rows.count == 201)
        #expect(model.hasMore == false)
        #expect(model.rows.last?.key == "word200")
    }

    @Test("filters by key prefix and restores the full page")
    func prefixSearch() {
        let (model, _) = makeModel(entries: [
            makeEntry("apple"), makeEntry("apricot"), makeEntry("banana")
        ])
        model.loadIfNeeded()
        model.searchText = "ap"
        model.applySearch()
        #expect(model.rows.map(\.key) == ["apple", "apricot"])

        model.clearSearch()
        #expect(model.rows.count == 3)
    }

    @Test("selects an entry and loads its form and diff")
    func selection() {
        let (model, fake) = makeModel()
        var diff = EntryDiff(
            key: "apple",
            kind: .modified,
            fields: [
                FieldDiff(field: "display", before: .string("Apple"), after: .string("apple"), changed: true),
                FieldDiff(field: "phonetic", before: nil, after: nil, changed: false)
            ]
        )
        diff = EntryDiff(key: "apple", kind: .modified, fields: diff.fields)
        fake.diffs["apple"] = diff
        model.loadIfNeeded()
        model.select("apple")

        #expect(model.form?.key == "apple")
        #expect(model.form?.display == "Apple")
        #expect(model.visibleDiffFields.map(\.field) == ["display"])

        model.showOnlyChanged = false
        #expect(model.visibleDiffFields.count == 2)
    }

    @Test("saves a valid form and refreshes the row badge")
    func savesEntry() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm { $0.display = "Apple (edited)" }
        #expect(model.save())

        #expect(fake.working["apple"]?.display == "Apple (edited)")
        #expect(model.rows.first { $0.key == "apple" }?.change == .modified)
        #expect(model.status.modified == 1)
        #expect(model.validationMessage == nil)
    }

    @Test("rejects an empty display and an edited key")
    func validatesForm() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")

        model.updateForm { $0.display = "   " }
        #expect(model.save() == false)
        #expect(model.validationMessage == "display 不能为空。")

        model.updateForm {
            $0.display = "Apple"
            $0.key = "renamed"
        }
        #expect(model.save() == false)
        #expect(model.validationMessage?.contains("key 不可") == true)
    }

    @Test("commit requires a message and changes")
    func commitValidation() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        #expect(model.commit(message: "nothing") == false)
        #expect(model.alertMessage == "工作区没有改动，不需要提交。")

        model.select("apple")
        model.updateForm { $0.display = "Apple 2" }
        _ = model.save()
        #expect(model.commit(message: "  ") == false)
        #expect(model.alertMessage == "提交信息不能为空。")
    }

    @Test("save and commit both notify the owner")
    func commitsChanges() {
        var commits = 0
        let (model, fake) = makeModel(onCommit: { commits += 1 })
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm { $0.display = "Apple 2" }
        // A save changes the working set, which the reader renders, so the
        // owner hears about it before anything is committed.
        #expect(model.save())
        #expect(commits == 1)

        #expect(model.commit(message: "edit apple"))
        #expect(fake.commitMessage == "edit apple")
        #expect(fake.commitAuthor == "tester")
        #expect(commits == 2)
        #expect(model.committed)
        #expect(model.hasChanges == false)
    }

    @Test("discard restores HEAD and clears selection")
    func discardsChanges() {
        let (model, fake) = makeModel(changes: [("banana", .modified)])
        model.loadIfNeeded()
        model.select("banana")
        model.updateForm { $0.display = "changed" }
        _ = model.save()
        #expect(model.hasChanges)

        #expect(model.discard())
        #expect(fake.discardCount == 1)
        #expect(model.hasChanges == false)
        #expect(model.selectedKey == nil)
        #expect(model.rows.first { $0.key == "banana" }?.change == nil)
        #expect(model.rows.first { $0.key == "banana" }?.display == "Banana")
    }

    @Test("deleted rows expose a diff without a form")
    func deletedRowDiff() {
        let (model, fake) = makeModel(changes: [("cherry", .deleted)])
        fake.diffs["cherry"] = EntryDiff(key: "cherry", kind: .deleted, fields: [])
        model.loadIfNeeded()
        model.select("cherry")

        #expect(model.form == nil)
        #expect(model.diff?.kind == .deleted)
    }

    // MARK: - Creating entries

    @Test("creates a placeholder entry, selects it and badges it as added")
    func createsEntry() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()

        #expect(model.createEntry())

        let key = "untitled"
        #expect(model.selectedKey == key)
        #expect(model.rows.contains { $0.key == key })
        #expect(model.rows.first { $0.key == key }?.change == .added)
        #expect(fake.working[key] != nil)
        #expect(model.form?.key == key)
        #expect(model.form?.display == "")
        #expect(model.form?.definitions.count == 1)
        #expect(model.canRenameSelectedEntry)
        #expect(model.isDirty)
        #expect(model.canUndo)
    }

    @Test("picks a free placeholder key when untitled is taken")
    func createsEntryWithFreeKey() {
        let (model, fake) = makeModel(entries: [makeEntry("untitled")])
        model.loadIfNeeded()

        #expect(model.createEntry())

        #expect(model.selectedKey == "untitled-2")
        #expect(fake.working["untitled-2"] != nil)
    }

    @Test("rejects an empty or duplicate key")
    func createEntryValidation() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()

        #expect(model.createEntry(key: "   ") == false)
        #expect(model.validationMessage == "key 不能为空。")

        #expect(model.createEntry(key: "apple") == false)
        #expect(model.validationMessage?.contains("已存在") == true)

        // Keys normalise the way storage does.
        #expect(model.createEntry(key: "Apple") == false)
        #expect(fake.working.count == 3)
    }

    @Test("renames a new entry when saving it")
    func renamesNewEntryOnSave() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        #expect(model.createEntry(key: "zebra"))

        model.updateForm { $0.display = "Zebra" }
        #expect(model.save())

        #expect(fake.working["zebra"]?.display == "Zebra")
        #expect(model.selectedKey == "zebra")
        #expect(model.form?.key == "zebra")
    }

    @Test("renames a new entry to another key on save")
    func renamesNewEntryKeyOnSave() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        #expect(model.createEntry(key: "zebra"))

        model.updateForm { form in
            form.key = "zebras"
            form.display = "Zebras"
        }
        #expect(model.save())

        #expect(fake.working["zebra"] == nil)
        #expect(fake.working["zebras"] != nil)
        #expect(model.selectedKey == "zebras")
    }

    @Test("keeps the key locked for entries that already existed")
    func rejectsRenameOfExistingEntry() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")

        model.updateForm { $0.key = "apples" }
        #expect(model.save() == false)
        #expect(model.validationMessage?.contains("key 不可") == true)
    }

    // MARK: - Deleting entries

    @Test("deletes an entry created in this session")
    func deletesNewEntry() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        #expect(model.createEntry(key: "zebra"))

        #expect(model.deleteEntry())

        #expect(fake.working["zebra"] == nil)
        #expect(!model.rows.contains { $0.key == "zebra" })
        #expect(!model.hasChanges)
        #expect(model.selectedKey == "cherry")
    }

    @Test("stages a deletion for a HEAD entry and moves the selection")
    func deletesHeadEntry() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        model.select("banana")

        #expect(model.deleteEntry())

        #expect(fake.working["banana"] == nil)
        #expect(model.rows.first { $0.key == "banana" }?.change == .deleted)
        #expect(model.status.removed == 1)
        #expect(model.selectedKey == "cherry")
        #expect(model.form?.key == "cherry")
    }

    // MARK: - Undo and redo

    @Test("undo restores a field edit and redo reapplies it")
    func undoRedoFieldEdit() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm("display") { $0.display = "Apple 2" }

        #expect(model.isDirty)
        #expect(model.canUndo)

        #expect(model.undo())
        #expect(model.form?.display == "Apple")
        #expect(!model.isDirty)
        #expect(model.canRedo)

        #expect(model.redo())
        #expect(model.form?.display == "Apple 2")
        #expect(model.isDirty)
    }

    @Test("undo reverts added and removed definitions")
    func undoDefinitions() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        let initial = model.form?.definitions.count ?? 0

        model.addDefinition()
        #expect(model.form?.definitions.count == initial + 1)

        #expect(model.undo())
        #expect(model.form?.definitions.count == initial)

        #expect(model.redo())
        #expect(model.form?.definitions.count == initial + 1)

        let removed = model.form?.definitions.first?.id
        model.removeDefinition(id: removed ?? UUID())
        #expect(model.form?.definitions.count == initial)

        #expect(model.undo())
        #expect(model.form?.definitions.contains { $0.id == removed } == true)
    }

    @Test("undo removes a created entry and redo brings it back")
    func undoCreate() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        #expect(model.createEntry(key: "zebra"))
        #expect(fake.working["zebra"] != nil)

        #expect(model.undo())
        #expect(fake.working["zebra"] == nil)
        #expect(!model.rows.contains { $0.key == "zebra" })
        #expect(model.selectedKey == nil)

        #expect(model.redo())
        #expect(fake.working["zebra"] != nil)
        #expect(model.rows.contains { $0.key == "zebra" })
        #expect(model.selectedKey == "zebra")
    }

    @Test("undo restores a deleted entry")
    func undoDelete() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        model.select("banana")
        #expect(model.deleteEntry())
        #expect(fake.working["banana"] == nil)

        #expect(model.undo())

        #expect(fake.working["banana"] != nil)
        #expect(model.selectedKey == "banana")
        #expect(model.form?.display == "Banana")
        #expect(!model.hasChanges)
        #expect(!model.isDirty)
    }

    @Test("undo after a save puts the stored entry back")
    func undoAfterSave() {
        let (model, fake) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm("display") { $0.display = "Apple 2" }
        #expect(model.save())
        #expect(fake.working["apple"]?.display == "Apple 2")

        #expect(model.undo())
        #expect(fake.working["apple"]?.display == "Apple")
        #expect(model.form?.display == "Apple 2")

        #expect(model.undo())
        #expect(model.form?.display == "Apple")
    }

    @Test("merges rapid edits of one field into a single step")
    func coalescesSameField() {
        let clock = TestClock()
        let (model, _) = makeModel(now: { clock.now })
        model.loadIfNeeded()
        model.select("apple")

        model.updateForm("display") { $0.display = "A" }
        clock.advance(0.2)
        model.updateForm("display") { $0.display = "Ap" }
        clock.advance(0.2)
        model.updateForm("display") { $0.display = "App" }

        #expect(model.undo())
        #expect(model.form?.display == "Apple")
        #expect(!model.canUndo)
    }

    @Test("starts a new step after the coalescing window or for another field")
    func splitsEdits() {
        let clock = TestClock()
        let (model, _) = makeModel(now: { clock.now })
        model.loadIfNeeded()
        model.select("apple")

        model.updateForm("display") { $0.display = "A" }
        clock.advance(1.0)
        model.updateForm("display") { $0.display = "AB" }
        model.updateForm("phoneticText") { $0.phoneticText = "x" }

        #expect(model.undo())
        #expect(model.form?.phoneticText == "")
        #expect(model.form?.display == "AB")

        #expect(model.undo())
        #expect(model.form?.display == "A")

        #expect(model.undo())
        #expect(model.form?.display == "Apple")
    }

    @Test("a new edit clears the redo stack")
    func newEditClearsRedo() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm("display") { $0.display = "A" }
        #expect(model.undo())
        #expect(model.canRedo)

        model.updateForm("display") { $0.display = "B" }

        #expect(!model.canRedo)
        #expect(model.redo() == false)
    }

    @Test("caps the undo stack")
    func capsUndoStack() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")

        for index in 0..<(DictionaryEditorModel.undoLimit + 20) {
            model.updateForm("field-\(index)") { $0.display = "v\(index)" }
        }

        var reverted = 0
        while model.undo() {
            reverted += 1
        }
        #expect(reverted == DictionaryEditorModel.undoLimit)
    }

    @Test("commit and discard clear the undo history")
    func clearsHistoryAfterCommitOrDiscard() {
        let (model, _) = makeModel()
        model.loadIfNeeded()
        model.select("apple")
        model.updateForm("display") { $0.display = "Apple 2" }
        #expect(model.save())
        #expect(model.canUndo)

        #expect(model.commit(message: "edit"))
        #expect(!model.canUndo)
        #expect(!model.canRedo)

        model.select("apple")
        model.updateForm("display") { $0.display = "Apple 3" }
        #expect(model.canUndo)

        #expect(model.discard())
        #expect(!model.canUndo)
        #expect(!model.canRedo)
    }
}

@Suite("Entry editor form")
struct EntryEditorFormTests {
    @Test("round-trips an entry and preserves extra fields")
    func roundTrip() {
        let extra: [String: JSONValue] = ["source": .string("csv")]
        let entry = DictionaryEntry(
            key: "bank",
            display: "Bank",
            phonetic: ["bæŋk"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a financial institution",
                    examples: ["Save at the bank"],
                    labels: ["finance"]
                )
            ],
            rawHTML: "<b>bank</b>",
            extra: extra
        )
        let rebuilt = EntryEditorForm(entry: entry).makeEntry(extra: entry.extra)
        #expect(rebuilt == entry)
    }

    @Test("splits examples, labels and phonetic fields")
    func splitsLists() {
        let form = EntryEditorForm(
            key: "key",
            display: "Display",
            phoneticText: "a, b",
            definitions: [
                DefinitionForm(
                    pos: "noun",
                    sense: "sense",
                    examplesText: "one\ntwo",
                    labelsText: "x, y"
                )
            ],
            rawHTML: ""
        )
        let entry = form.makeEntry(extra: [:])
        #expect(entry.phonetic == ["a", "b"])
        #expect(entry.defs.first?.examples == ["one", "two"])
        #expect(entry.defs.first?.labels == ["x", "y"])
        #expect(entry.rawHTML == nil)
    }
}

/// Deterministic clock for the undo coalescing window.
@MainActor
private final class TestClock {
    private(set) var now = Date(timeIntervalSince1970: 1_000)

    func advance(_ seconds: TimeInterval) {
        now = now.addingTimeInterval(seconds)
    }
}
