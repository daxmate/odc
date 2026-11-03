import Foundation
import Testing
@testable import ODictKit

@Suite("Dictionary editor model")
@MainActor
struct DictionaryEditorModelTests {
    private func makeModel(
        entries: [DictionaryEntry] = [makeEntry("apple"), makeEntry("banana"), makeEntry("cherry")],
        changes: [(String, ChangeKind)] = [],
        onCommit: (() -> Void)? = nil
    ) -> (DictionaryEditorModel, FakeEditing) {
        let fake = FakeEditing(entries: entries, changes: changes)
        let model = DictionaryEditorModel(editing: fake, author: "tester", onCommit: onCommit)
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
