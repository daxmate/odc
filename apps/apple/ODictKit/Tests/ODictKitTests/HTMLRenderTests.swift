import WebKit
import XCTest
@testable import ODictKit

/// Drives the entry reader's web view end to end: real HTML, the real
/// odc-resource scheme handler, real shadow roots.
@MainActor
final class HTMLRenderTests: XCTestCase {
    // MARK: - The blank entry regression

    /// A dictionary that links its stylesheet as a resource (LDOCE6++ style).
    /// The document's base URL is odc-resource:///, so the reader's navigation
    /// policy must not cancel the load it started itself.
    func testEntryWithStylesheetServedFromResourcesShowsItsBody() async throws {
        let renderer = HTMLRenderer(resources: { _, key in
            key == "ldoce6ec.css" ? Data("p { color: #111; }".utf8) : nil
        })

        renderer.load(document: Self.page("""
        <link rel="stylesheet" href="ldoce6ec.css">
        <p class="entry">hello world</p>
        """))

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    func testEntryWithInlineStyleShowsItsBody() async throws {
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        renderer.load(document: Self.page("""
        <style>p { color: #111; }</style>
        <p class="entry">hello world</p>
        """))

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    /// A stylesheet that cannot be served must not take the entry down with it.
    func testEntryWithMissingStylesheetStillShowsItsBody() async throws {
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        renderer.load(document: Self.page("""
        <link rel="stylesheet" href="missing.css">
        <p class="entry">hello world</p>
        """))

        let text = await Self.bodyText(in: renderer.webView)
        XCTAssertTrue(text.contains("hello world"), "body text was: \(text)")
    }

    // MARK: - Dictionary stylesheets

    /// A dictionary that keeps its CSS in the repository: the reader inlines
    /// it, so the entry is styled even though nothing is fetched over the wire.
    func testInlinesTheDictionaryStylesheet() async throws {
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        let section = EntryHTML.PageSection(
            dictionaryID: "dict",
            name: "Dictionary",
            html: "<p class=\"dict\">styled</p>",
            stylesheets: [EntryHTML.LinkedStylesheet(key: "style.css", css: "p.dict { color: rgb(255, 0, 0); }")]
        )
        renderer.load(document: EntryHTML.page([section]))

        var colour: String?
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            colour = await Self.evaluate(
                "document.querySelector('.odc-section').shadowRoot"
                    + ".querySelector('p.dict')"
                    + " ? getComputedStyle(document.querySelector('.odc-section').shadowRoot"
                    + ".querySelector('p.dict')).color : null",
                in: renderer.webView
            )
            if colour == "rgb(255, 0, 0)" { break }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        XCTAssertEqual(colour, "rgb(255, 0, 0)")
    }

    // MARK: - Outcome reporting (drives the structured fallback)

    func testReportsVisibleCharactersOnceLoaded() async throws {
        let box = OutcomeBox()
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        renderer.onOutcome = { box.record($0) }
        renderer.load(document: Self.page("<p>hello world</p>"))

        let outcome = await Self.waitForOutcome(box)
        guard case .loaded(let characters) = outcome else {
            return XCTFail("outcome was: \(String(describing: outcome))")
        }
        XCTAssertGreaterThan(characters, 0)
    }

    /// The text lives inside a shadow root now, where body.innerText stops.
    /// Counting must cross the boundary or every entry would look blank.
    func testCountsTextInsideShadowRoots() async throws {
        let box = OutcomeBox()
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        renderer.onOutcome = { box.record($0) }
        renderer.load(document: Self.page("<p>shadow text</p>"))

        let outcome = await Self.waitForOutcome(box)
        XCTAssertEqual(outcome, .loaded(visibleCharacters: 11), "outcome was: \(String(describing: outcome))")
    }

    func testReportsEmptyDocumentWhenNothingIsVisible() async throws {
        let box = OutcomeBox()
        let renderer = HTMLRenderer(resources: { _, _ in nil })
        renderer.onOutcome = { box.record($0) }
        renderer.load(document: Self.page("<p></p>"))

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
        let handler = ODCResourceSchemeHandler(resources: { _, _ in nil })
        let task = RecordingSchemeTask(url: URL(string: "odc-resource:///missing.png")!)

        handler.webView(WKWebView(), start: task)

        XCTAssertEqual((task.responses.first as? HTTPURLResponse)?.statusCode, 404)
        XCTAssertEqual(task.finished, 1)
        XCTAssertTrue(task.failures.isEmpty)
    }

    func testSchemeHandlerServesThePayloadExactlyOnce() {
        let png = Data([0x89, 0x50, 0x4E, 0x47])
        let handler = ODCResourceSchemeHandler(resources: { _, key in
            key == "pic/a.png" ? png : nil
        })
        let task = RecordingSchemeTask(url: URL(string: "odc-resource:///pic/a.png")!)

        handler.webView(WKWebView(), start: task)

        XCTAssertEqual(task.responses.count, 1)
        XCTAssertEqual(task.payload, png)
        XCTAssertEqual(task.finished, 1)
        XCTAssertTrue(task.failures.isEmpty)
    }

    /// A page that stacks dictionaries names the section in the URL, and the
    /// handler hands that section back to the caller.
    func testSchemeHandlerReportsTheRequestingSection() {
        var asked: [Int] = []
        let handler = ODCResourceSchemeHandler(resources: { section, key in
            asked.append(section)
            return key == "pic/a.png" ? Data([0x1]) : nil
        })
        let task = RecordingSchemeTask(url: URL(string: "odc-resource:///s2/pic/a.png")!)

        handler.webView(WKWebView(), start: task)

        XCTAssertEqual(asked, [2])
        XCTAssertEqual(task.finished, 1)
    }

    // MARK: - Helpers

    /// A one-dictionary page, the shape the reader hands the web view.
    static func page(_ html: String) -> String {
        EntryHTML.page([
            EntryHTML.PageSection(dictionaryID: "dict", name: "Dictionary", html: html),
        ])
    }

    /// Visible text, walking into each section's shadow root.
    static func bodyText(in webView: WKWebView, timeout: TimeInterval = 10) async -> String {
        let script = """
        (function () {
          function text(node) {
            var out = "";
            var children = node.childNodes || [];
            for (var i = 0; i < children.length; i++) {
              var child = children[i];
              if (child.nodeType === 3) { out += child.nodeValue || ""; continue; }
              if (child.nodeType !== 1) { continue; }
              var tag = (child.tagName || "").toUpperCase();
              if (tag === "STYLE" || tag === "SCRIPT" || tag === "TEMPLATE" || tag === "LINK") { continue; }
              if (child.shadowRoot) { out += text(child.shadowRoot); }
              out += text(child);
            }
            return out;
          }
          return text(document.body);
        })()
        """
        let deadline = Date().addingTimeInterval(timeout)
        var last = ""
        while Date() < deadline {
            if let value = await evaluate(script, in: webView) {
                last = value
                if !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    return value
                }
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        return last
    }

    fileprivate static func waitForOutcome(_ box: OutcomeBox, timeout: TimeInterval = 10) async -> EntryRenderOutcome? {
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
