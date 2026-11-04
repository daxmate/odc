import Foundation
import ODictCore
import XCTest

/// Exercises the HTML check / repair binding through the real C ABI.
final class ODCHTMLTests: XCTestCase {
    private let malformed = "<p>hi</p><script>alert(1)</script>"

    func testCheckReportsDiagnosticsWithoutChangingHTML() throws {
        let result = try OdcHTML.process(malformed, mode: .check)
        XCTAssertEqual(result.html, malformed)
        XCTAssertFalse(result.changed)
        XCTAssertTrue(
            result.diagnostics.contains { $0.code == "html.removed_element" },
            "script removal must be reported: \(result.diagnostics)"
        )
        XCTAssertFalse(result.diagnostics.contains { $0.message.isEmpty })
    }

    func testFormatRepairsHTML() throws {
        let result = try OdcHTML.process(malformed, mode: .format)
        XCTAssertTrue(result.changed)
        XCTAssertFalse(result.html.contains("script"), result.html)
        XCTAssertTrue(result.html.contains("hi"), result.html)
    }

    func testEmptyBodyIsAccepted() throws {
        let result = try OdcHTML.process("", mode: .check)
        XCTAssertEqual(result.html, "")
        XCTAssertFalse(result.changed)
        XCTAssertTrue(result.diagnostics.isEmpty)
    }
}
