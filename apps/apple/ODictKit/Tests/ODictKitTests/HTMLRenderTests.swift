import WebKit
import XCTest
@testable import ODictKit

/// Drives the entry reader's web view end to end: real HTML, the real
/// `odc-resource` scheme handler, real `document.body.innerText`.
@MainActor
final class HTMLRenderTests: XCTestCase {
    // MARK: - The blank entry regression

    /// A dictionary that links its stylesheet as a resource (LDOCE6++ style).
    /// The document's base URL is `odc-resource:///`, so the reader's
    /// navigation policy must not cancel the load it started itself.
    func testEntryWithStylesheetServedFromResourcesShowsItsBody() async throws {
        let renderer = HTMLRenderer(resources: { key in
            key == "ldoce6ec.css" ? Data("p { color: #111; }".utf8) : nil
        })

        renderer.load(html: """
        <link rel="stylesheet" href="ldoce6ec.css">
        <p class="entry">hello world</p>
        """)

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    /// A dictionary whose typography is inline (牛津高阶 style) needs no
    /// resource at all; it must keep working.
    func testEntryWithInlineStyleShowsItsBody() async throws {
        let renderer = HTMLRenderer(resources: { _ in nil })
        renderer.load(html: """
        <style>p { color: #111; }</style>
        <p class="entry">hello world</p>
        """)

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    /// A stylesheet key the volume does not carry answers 404; the entry is
    /// still readable.
    func testMissingStylesheetStillShowsBody() async throws {
        let renderer = HTMLRenderer(resources: { _ in nil })
        renderer.load(html: """
        <link rel="stylesheet" href="missing.css">
        <p class="entry">hello world</p>
        """)

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    // MARK: - Outcome reporting (drives the structured fallback)

    func testReportsVisibleCharactersOnceLoaded() async throws {
        let box = OutcomeBox()
        let renderer = HTMLRenderer(resources: { _ in nil })
        renderer.onOutcome = { box.record($0) }
        renderer.load(html: "<p class=\"entry\">hello world</p>")

        let outcome = await Self.waitForOutcome(box)
        guard case .loaded(let characters) = outcome else {
            return XCTFail("expected a loaded outcome, got \(String(describing: outcome))")
        }
        XCTAssertGreaterThanOrEqual(characters, "hello world".count)
    }

    func testReportsEmptyDocumentWhenNothingIsVisible() async throws {
        let box = OutcomeBox()
        let renderer = HTMLRenderer(resources: { _ in nil })
        renderer.onOutcome = { box.record($0) }
        renderer.load(html: "<p></p>")

        let outcome = await Self.waitForOutcome(box)
        XCTAssertEqual(outcome, .loaded(visibleCharacters: 0))
    }

    func testFallbackMessageExplainsAFailedLoad() {
        XCTAssertNil(EntryDetailView.fallbackMessage(for: .loaded(visibleCharacters: 12)))
        XCTAssertNotNil(EntryDetailView.fallbackMessage(for: .loaded(visibleCharacters: 0)))
        let message = EntryDetailView.fallbackMessage(for: .failed("boom"))
        XCTAssertTrue(message?.contains("boom") == true, "message was: \(String(describing: message))")
    }

    // MARK: - Scheme handler

    func testSchemeHandlerAnswersAMissingKeyWith404AndFinishes() {
        let handler = ODCResourceSchemeHandler(resources: { _ in nil })
        let task = RecordingSchemeTask(url: URL(string: "odc-resource:///missing.png")!)

        handler.webView(WKWebView(), start: task)

        XCTAssertEqual((task.responses.first as? HTTPURLResponse)?.statusCode, 404)
        XCTAssertEqual(task.finished, 1)
        XCTAssertTrue(task.failures.isEmpty)
    }

    func testSchemeHandlerServesThePayloadExactlyOnce() {
        let png = Data([0x89, 0x50, 0x4E, 0x47])
        let handler = ODCResourceSchemeHandler(resources: { key in
            key == "images/a.png" ? png : nil
        })
        let task = RecordingSchemeTask(url: URL(string: "odc-resource:///images/a.png")!)

        handler.webView(WKWebView(), start: task)

        XCTAssertEqual(task.responses.count, 1)
        XCTAssertEqual(task.responses.first?.mimeType, "image/png")
        XCTAssertEqual(task.payload, png)
        XCTAssertEqual(task.finished, 1)
        XCTAssertTrue(task.failures.isEmpty)
    }
}

// MARK: - Helpers

@MainActor
private final class OutcomeBox {
    private(set) var outcome: EntryRenderOutcome?

    func record(_ outcome: EntryRenderOutcome) {
        self.outcome = outcome
    }
}

/// Records what the scheme handler hands to WebKit. The handler calls back on
/// the main actor; the type itself stays non-isolated to satisfy the protocol.
private final class RecordingSchemeTask: NSObject, WKURLSchemeTask, @unchecked Sendable {
    let request: URLRequest
    private(set) var responses: [URLResponse] = []
    private(set) var payload = Data()
    private(set) var finished = 0
    private(set) var failures: [any Error] = []

    init(url: URL) {
        request = URLRequest(url: url)
        super.init()
    }

    func didReceive(_ response: URLResponse) {
        responses.append(response)
    }

    func didReceive(_ data: Data) {
        payload.append(data)
    }

    func didFinish() {
        finished += 1
    }

    func didFailWithError(_ error: any Error) {
        failures.append(error)
    }
}

extension HTMLRenderTests {
    /// Polls `document.body.innerText` until it is non-empty or `timeout` runs out.
    static func bodyText(in webView: WKWebView, timeout: TimeInterval = 15) async -> String {
        let deadline = Date().addingTimeInterval(timeout)
        var last = ""
        while Date() < deadline {
            if let value = await evaluate("document.body ? document.body.innerText : ''", in: webView) {
                last = value
                if !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    return value
                }
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        return last
    }

    fileprivate static func waitForOutcome(
        _ box: OutcomeBox,
        timeout: TimeInterval = 15
    ) async -> EntryRenderOutcome? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let outcome = box.outcome { return outcome }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        return box.outcome
    }

    static func evaluate(_ script: String, in webView: WKWebView) async -> String? {
        await withCheckedContinuation { continuation in
            webView.evaluateJavaScript(script) { value, _ in
                continuation.resume(returning: value as? String)
            }
        }
    }
}
