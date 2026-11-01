#if os(macOS)
import AppKit
import XCTest
@testable import ODictKit

@MainActor
final class SearchFieldReturnTests: XCTestCase {
    func testSearchFieldEditorIsRecognised() throws {
        let (window, field) = try makeWindow(
            with: NSSearchField(frame: .init(x: 0, y: 0, width: 200, height: 24))
        )
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertTrue(SearchFieldResponder.isSearchFieldEditor(window.firstResponder))
    }

    func testPlainTextFieldIsIgnored() throws {
        let (window, field) = try makeWindow(
            with: NSTextField(frame: .init(x: 0, y: 0, width: 200, height: 24))
        )
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertFalse(SearchFieldResponder.isSearchFieldEditor(window.firstResponder))
        XCTAssertFalse(SearchFieldResponder.shouldSubmit(responder: window.firstResponder))
    }

    /// Return belongs to the input method until the candidate is accepted.
    func testReturnIsNotTakenDuringComposition() throws {
        let (window, field) = try makeWindow(
            with: NSSearchField(frame: .init(x: 0, y: 0, width: 200, height: 24))
        )
        XCTAssertTrue(window.makeFirstResponder(field))
        let editor = try XCTUnwrap(window.firstResponder as? NSTextView)
        XCTAssertTrue(SearchFieldResponder.shouldSubmit(responder: editor))

        editor.setMarkedText(
            "zhong",
            selectedRange: NSRange(location: 5, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertTrue(editor.hasMarkedText())
        XCTAssertTrue(SearchFieldResponder.isSearchFieldEditor(editor), "still the search box")
        XCTAssertFalse(SearchFieldResponder.shouldSubmit(responder: editor), "but the IME owns Return")
    }

    func testNoResponderIsIgnored() {
        XCTAssertFalse(SearchFieldResponder.isSearchFieldEditor(nil))
    }

    /// An NSTextView that is not a field editor (a text pane, say) is not the
    /// search box either.
    func testStandaloneTextViewIsIgnored() {
        XCTAssertFalse(SearchFieldResponder.isSearchFieldEditor(NSTextView()))
    }

    private func makeWindow(with control: NSControl) throws -> (NSWindow, NSControl) {
        let window = NSWindow(
            contentRect: .init(x: 0, y: 0, width: 300, height: 200),
            styleMask: [.titled],
            backing: .buffered,
            defer: false
        )
        let container = NSView(frame: .init(x: 0, y: 0, width: 300, height: 200))
        window.contentView = container
        container.addSubview(control)
        return (window, control)
    }
}
#endif
