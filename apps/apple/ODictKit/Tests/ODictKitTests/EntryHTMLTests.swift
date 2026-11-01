import Foundation
import XCTest
@testable import ODictKit

final class EntryHTMLTests: XCTestCase {
    // MARK: - document(html:)

    func testDocumentHasDoctypeCharsetAndViewport() {
        let document = EntryHTML.document(html: "<p>hello</p>")
        XCTAssertTrue(document.hasPrefix("<!DOCTYPE html>"))
        XCTAssertTrue(document.contains("charset=\"utf-8\""))
        XCTAssertTrue(document.contains("name=\"viewport\""))
    }

    func testDocumentCarriesTheDefaultStylesheet() {
        let document = EntryHTML.document(html: "")
        XCTAssertTrue(document.contains("color-scheme: light dark"))
        XCTAssertTrue(document.contains("-apple-system"))
        XCTAssertTrue(document.contains("line-height: 1.5"))
        XCTAssertTrue(document.contains("margin: 16px"))
        XCTAssertTrue(document.contains("max-width: 100%"))
        XCTAssertTrue(document.contains("prefers-color-scheme: dark"))
        XCTAssertTrue(document.contains("word-break"))
    }

    func testDocumentKeepsTheEntryMarkup() {
        let entry = "<div class=\"entry\"><p>apple</p></div>"
        let document = EntryHTML.document(html: entry)
        XCTAssertTrue(document.contains(entry))
    }

    func testDocumentKeepsTheEntryStyleBlock() {
        let entry = "<style>.entry { color: red; }</style><p class=\"entry\">apple</p>"
        let document = EntryHTML.document(html: entry)
        XCTAssertTrue(document.contains("<style>.entry { color: red; }</style>"))
    }

    func testDocumentDropsScriptsOnTheWayIn() {
        let document = EntryHTML.document(html: "<p>apple</p><script>alert(1)</script>")
        XCTAssertFalse(document.contains("alert(1)"))
        XCTAssertTrue(document.contains("<p>apple</p>"))
    }

    // MARK: - sanitized(_:)

    func testSanitizedRemovesScriptElements() {
        let html = "<p>a</p><script>alert(1)</script><p>b</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p><p>b</p>")
    }

    func testSanitizedIgnoresScriptCase() {
        let html = "<p>a</p><SCRIPT>alert(1)</SCRIPT>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesScriptWithSource() {
        let html = "<p>a</p><script src=\"evil.js\"></script>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesSelfClosingScript() {
        let html = "<p>a</p><script src=\"evil.js\" />"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesMultiLineScript() {
        let html = """
        <p>a</p>
        <script type="text/javascript">
        var x = 1;
        alert(x);
        </script>
        <p>b</p>
        """
        let sanitized = EntryHTML.sanitized(html)
        XCTAssertFalse(sanitized.contains("alert"))
        XCTAssertFalse(sanitized.lowercased().contains("<script"))
        XCTAssertTrue(sanitized.contains("<p>a</p>"))
        XCTAssertTrue(sanitized.contains("<p>b</p>"))
    }

    func testSanitizedRemovesEventHandlers() {
        let html = "<p onclick=\"alert(1)\" onmouseover='x()' class=\"keep\">text</p>"
        let sanitized = EntryHTML.sanitized(html)
        XCTAssertFalse(sanitized.contains("alert(1)"))
        XCTAssertFalse(sanitized.contains("x()"))
        XCTAssertFalse(sanitized.lowercased().contains("onclick"))
        XCTAssertFalse(sanitized.lowercased().contains("onmouseover"))
        XCTAssertTrue(sanitized.contains("class=\"keep\""))
        XCTAssertTrue(sanitized.contains("text"))
    }

    func testSanitizedKeepsAttributesThatMerelyMentionScript() {
        let html = "<span title=\"script\" data-note=\"use a script here\">text</span>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedKeepsOrdinaryAttributesStartingWithOn() {
        let html = "<div only=\"one\" once=\"twice\">text</div>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedLeavesCommentsAlone() {
        let html = "<!-- <script>alert(1)</script> --><p>a</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedKeepsStyleBlocks() {
        let html = "<style>p { color: red; }</style><p>a</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedLeavesPlainTextUntouched() {
        let html = "<p>apple — 苹果</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    // MARK: - entryTarget(from:)

    func testEntryTargetStripsScheme() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple")!), "apple")
    }

    func testEntryTargetDecodesPercentEscapes() {
        XCTAssertEqual(
            EntryHTML.entryTarget(from: URL(string: "entry://%E8%8B%B9%E6%9E%9C")!),
            "苹果"
        )
    }

    func testEntryTargetDropsTheFragment() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple#n.")!), "apple")
    }

    func testEntryTargetDropsTheQuery() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple?x=1")!), "apple")
    }

    func testEntryTargetIsNilWithoutAKey() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "entry://")!))
    }

    func testEntryTargetIsNilForAnEmptyFragmentOnly() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "entry://#n.")!))
    }

    func testEntryTargetPreservesCase() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://Apple")!), "Apple")
    }

    func testEntryTargetIsNilForOtherSchemes() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "https://example.com/apple")!))
    }
}
