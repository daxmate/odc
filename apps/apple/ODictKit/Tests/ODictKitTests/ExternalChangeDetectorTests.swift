import Foundation
import Testing
@testable import ODictKit

@Suite("External change detection")
struct ExternalChangeDetectorTests {
    @Test("stays in sync while HEAD matches")
    func inSync() {
        var detector = ExternalChangeDetector(baselineHead: "a")
        #expect(detector.evaluate(head: "a", hasLocalChanges: false) == .inSync)
        #expect(detector.evaluate(head: "a", hasLocalChanges: true) == .inSync)
    }

    @Test("reports a clean reload once and adopts the new head")
    func cleanReload() {
        var detector = ExternalChangeDetector(baselineHead: "a")
        #expect(detector.evaluate(head: "b", hasLocalChanges: false) == .reloaded)
        #expect(detector.baselineHead == "b")
        #expect(detector.evaluate(head: "b", hasLocalChanges: false) == .inSync)
    }

    @Test("keeps the baseline while local work is at risk")
    func conflict() {
        var detector = ExternalChangeDetector(baselineHead: "a")
        #expect(detector.evaluate(head: "b", hasLocalChanges: true) == .conflict)
        // Still the old baseline: the caller has not decided yet.
        #expect(detector.baselineHead == "a")
        #expect(detector.evaluate(head: "b", hasLocalChanges: true) == .conflict)
    }

    @Test("adopt re-baselines without reporting a change")
    func adopt() {
        var detector = ExternalChangeDetector(baselineHead: "a")
        detector.adopt(head: "b")
        #expect(detector.baselineHead == "b")
        #expect(detector.evaluate(head: "b", hasLocalChanges: true) == .inSync)
    }

    @Test("a first load onto a real head is not a change")
    func firstLoad() {
        var detector = ExternalChangeDetector()
        detector.adopt(head: nil)
        #expect(detector.evaluate(head: nil, hasLocalChanges: false) == .inSync)
    }
}

@Suite("Editor external updates")
@MainActor
struct EditorExternalChangeTests {
    private func makeEditor(
        entries: [DictionaryEntry] = [makeEntry("apple")]
    ) -> (DictionaryEditorModel, FakeEditing) {
        let fake = FakeEditing(entries: entries)
        let model = DictionaryEditorModel(editing: fake, author: "tester")
        model.loadIfNeeded()
        return (model, fake)
    }

    @Test("reloads silently when the working set is clean")
    func silentReload() {
        let (model, fake) = makeEditor()
        fake.headRef = "foreign"
        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange == false)
        #expect(model.noticeMessage == "库已被其他进程更新，已重新加载。")

        // The foreign head is adopted: checking again is quiet.
        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange == false)
    }

    @Test("asks before touching a dirty working set")
    func conflictPrompt() {
        let (model, fake) = makeEditor()
        fake.working["apple"] = makeEntry("apple", display: "Apricot")
        fake.headRef = "foreign"

        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange)
        #expect(model.hasChanges)

        model.keepLocalChanges()
        #expect(model.hasPendingExternalChange == false)
        #expect(model.hasChanges)

        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange == false)
    }

    @Test("discarding local work adopts the foreign head")
    func discardAfterConflict() {
        let (model, fake) = makeEditor()
        fake.working["apple"] = makeEntry("apple", display: "Apricot")
        fake.headRef = "foreign"
        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange)

        model.discardLocalChangesAfterExternalChange()
        #expect(fake.discardCount == 1)
        #expect(model.hasPendingExternalChange == false)
        #expect(model.hasChanges == false)

        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange == false)
    }

    @Test("a dismissed prompt asks again on the next check")
    func dismissedPrompt() {
        let (model, fake) = makeEditor()
        fake.working["apple"] = makeEntry("apple", display: "Apricot")
        fake.headRef = "foreign"
        model.checkForExternalChanges()
        model.pendingExternalChangeDismissed()
        #expect(model.hasPendingExternalChange == false)

        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange)
    }

    @Test("a commit through the panel shows the short hash")
    func didCommit() {
        let (model, fake) = makeEditor()
        fake.working["banana"] = makeEntry("banana")
        fake.headRef = "abc1234def5678"
        model.didCommit(hash: "abc1234def5678")
        #expect(model.noticeMessage == "已提交 abc1234")
        model.checkForExternalChanges()
        #expect(model.hasPendingExternalChange == false)
    }

    @Test("the panel model shares the editor's repository")
    func panelModel() {
        let (model, fake) = makeEditor()
        fake.working["banana"] = makeEntry("banana")
        let panel = model.makeWorkingChangesModel()
        panel.refresh()
        #expect(panel.status.added == 1)
        #expect(panel.sections.map(\.kind) == [.added])
    }
}
