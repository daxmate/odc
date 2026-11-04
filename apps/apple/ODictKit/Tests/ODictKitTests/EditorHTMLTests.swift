import Foundation
import ODictCore
import Testing
@testable import ODictKit

@Suite("Editor HTML check and repair")
@MainActor
struct EditorHTMLTests {
    /// Records the calls the editor makes; never touches the C ABI.
    private final class StubHTMLProcessor: ODCHTMLProcessing, @unchecked Sendable {
        struct Call: Equatable {
            let html: String
            let mode: HtmlCheckMode
        }

        var calls: [Call] = []
        let result: HtmlProcessResult
        let error: OdcError?

        init(result: HtmlProcessResult, error: OdcError? = nil) {
            self.result = result
            self.error = error
        }

        func processHTML(_ html: String, mode: HtmlCheckMode) throws -> HtmlProcessResult {
            calls.append(Call(html: html, mode: mode))
            if let error {
                throw error
            }
            return result
        }
    }

    private func makeModel(
        rawHTML: String,
        processor: StubHTMLProcessor
    ) -> DictionaryEditorModel {
        let entry = makeEntry("apple", rawHTML: rawHTML)
        let fake = FakeEditing(entries: [entry])
        let model = DictionaryEditorModel(
            editing: fake,
            author: "tester",
            htmlProcessor: processor
        )
        model.select("apple")
        return model
    }

    @Test("check reports diagnostics without touching the form")
    func checkReportsDiagnostics() {
        let diagnostic = HtmlDiagnostic(code: "html.parse_error", message: "unexpected end tag")
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "<div>", changed: false, diagnostics: [diagnostic])
        )
        let model = makeModel(rawHTML: "<div>", processor: stub)

        model.checkSelectedEntryHTML()

        #expect(model.htmlDiagnostics == [diagnostic])
        #expect(model.htmlChecked)
        #expect(model.htmlCheckMessage == nil)
        #expect(model.form?.rawHTML == "<div>")
        #expect(model.canUndo == false)
        #expect(stub.calls == [.init(html: "<div>", mode: .check)])
    }

    @Test("check on an empty body explains instead of calling the ABI")
    func checkEmptyBody() {
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "", changed: false, diagnostics: [])
        )
        let model = makeModel(rawHTML: "   ", processor: stub)

        model.checkSelectedEntryHTML()

        #expect(model.htmlChecked)
        #expect(model.htmlCheckMessage == "这个词条没有 HTML 内容。")
        #expect(model.htmlDiagnostics.isEmpty)
        #expect(stub.calls.isEmpty)
    }

    @Test("check without problems shows the empty state")
    func checkWithoutProblems() {
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "<p>hi</p>", changed: false, diagnostics: [])
        )
        let model = makeModel(rawHTML: "<p>hi</p>", processor: stub)

        model.checkSelectedEntryHTML()

        #expect(model.htmlChecked)
        #expect(model.htmlCheckMessage == "没有发现问题。")
        #expect(model.htmlDiagnostics.isEmpty)
    }

    @Test("fix rewrites raw_html as one undoable step")
    func fixWritesBackThroughUpdateForm() {
        let repaired = HtmlProcessResult(
            html: "<div><p>hi</p></div>",
            changed: true,
            diagnostics: [HtmlDiagnostic(code: "html.parse_error", message: "unclosed")]
        )
        let stub = StubHTMLProcessor(result: repaired)
        let model = makeModel(rawHTML: "<div><p>hi", processor: stub)

        #expect(model.applyHTMLFix())

        #expect(model.form?.rawHTML == "<div><p>hi</p></div>")
        #expect(model.htmlDiagnostics.isEmpty)
        #expect(model.htmlCheckMessage == "已修复 HTML，保存到工作区后生效。")
        #expect(model.canUndo)

        #expect(model.undo())
        #expect(model.form?.rawHTML == "<div><p>hi")
        #expect(model.canUndo == false)
    }

    @Test("fix without changes does not add an undo step")
    func fixWithoutChangesKeepsHistory() {
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "<p>hi</p>", changed: false, diagnostics: [])
        )
        let model = makeModel(rawHTML: "<p>hi</p>", processor: stub)

        #expect(model.applyHTMLFix())

        #expect(model.form?.rawHTML == "<p>hi</p>")
        #expect(model.htmlCheckMessage == "没有需要修复的问题。")
        #expect(model.canUndo == false)
    }

    @Test("fix failures surface as an alert and leave the form alone")
    func fixFailureKeepsForm() {
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "", changed: false, diagnostics: []),
            error: .htmlFailed("boom")
        )
        let model = makeModel(rawHTML: "<div>", processor: stub)

        #expect(model.applyHTMLFix() == false)

        #expect(model.form?.rawHTML == "<div>")
        #expect(model.alertMessage?.contains("HTML 处理失败") == true)
        #expect(model.canUndo == false)
    }

    @Test("selecting another entry clears stale diagnostics")
    func selectClearsDiagnostics() {
        let diagnostic = HtmlDiagnostic(code: "html.parse_error", message: "unexpected end tag")
        let stub = StubHTMLProcessor(
            result: HtmlProcessResult(html: "<div>", changed: false, diagnostics: [diagnostic])
        )
        let entry = makeEntry("apple", rawHTML: "<div>")
        let other = makeEntry("banana", rawHTML: "<p>ok</p>")
        let fake = FakeEditing(entries: [entry, other])
        let model = DictionaryEditorModel(
            editing: fake,
            author: "tester",
            htmlProcessor: stub
        )
        model.select("apple")
        model.checkSelectedEntryHTML()
        #expect(!model.htmlDiagnostics.isEmpty)

        model.select("banana")

        #expect(model.htmlDiagnostics.isEmpty)
        #expect(model.htmlChecked == false)
        #expect(model.htmlCheckMessage == nil)
    }
}
