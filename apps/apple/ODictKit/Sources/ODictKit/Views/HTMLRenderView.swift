import AVFoundation
import os
import SwiftUI
import WebKit

#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// What the reader got from the web view for the document it just loaded.
public enum EntryRenderOutcome: Equatable, Sendable {
    /// The document finished loading and had this many characters of visible text.
    case loaded(visibleCharacters: Int)
    /// The document never loaded; the caller should fall back to the parsed entry.
    case failed(String)
}

/// Navigation diagnostics for the entry reader: watch them with
/// `log stream --predicate 'subsystem == "com.daxmate.odict" && category == "render"'`.
private let renderLog = Logger(subsystem: "com.daxmate.odict", category: "render")

/// Shows an entry's HTML exactly as the dictionary wrote it, in a web view that
/// cannot run scripts.
public struct HTMLRenderView: View {
    private let html: String
    private let resources: @MainActor @Sendable (String) -> Data?
    private let onEntryLink: @MainActor @Sendable (String) -> Void
    private let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?
    /// Reports how tall the document is, so a page can stack several entries
    /// instead of giving each one a scrolling window of its own.
    private let onContentHeight: (@MainActor (CGFloat) -> Void)?

    /// - Parameters:
    ///   - html: The entry's raw HTML.
    ///   - resources: Payload for one MDD key, `nil` when the dictionary has
    ///     none. Served under the `odc-resource` scheme.
    ///   - onEntryLink: Called with a target key when a link uses `entry://`.
    public init(
        html: String,
        resources: @escaping @MainActor @Sendable (String) -> Data? = { _ in nil },
        onEntryLink: @escaping @MainActor @Sendable (String) -> Void = { _ in },
        onOutcome: (@MainActor (EntryRenderOutcome) -> Void)? = nil,
        onContentHeight: (@MainActor (CGFloat) -> Void)? = nil
    ) {
        self.html = html
        self.resources = resources
        self.onEntryLink = onEntryLink
        self.onOutcome = onOutcome
        self.onContentHeight = onContentHeight
    }

    public var body: some View {
        HTMLRenderRepresentable(
            html: html,
            resources: resources,
            onEntryLink: onEntryLink,
            onOutcome: onOutcome,
            onContentHeight: onContentHeight
        )
    }
}

/// Owns the one web view a renderer instance reuses, and routes its link taps.
@MainActor
final class HTMLRenderer: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    var onEntryLink: (@MainActor @Sendable (String) -> Void)?
    /// Reports the document's fate so the reader can replace a blank web view.
    var onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?
    /// Reports the document's height when someone is stacking entries.
    var onContentHeight: (@MainActor (CGFloat) -> Void)?
    /// Width the last reported height was measured at; a resize invalidates it.
    private var measuredWidth: CGFloat?
    /// Last document handed to the web view; reloading the same HTML would only
    /// make SwiftUI redraws flicker.
    private var loadedHTML: String?
    /// Payload source for `odc-resource` links and the scheme handler.
    private let resources: @MainActor (String) -> Data?
    /// Speaker icons are links to an audio clip; the reader plays them in
    /// place instead of navigating to a media file.
    private let audio = ResourceAudioPlayer()

    /// Hands a tapped http(s) link to the system browser.
    private func openExternally(_ url: URL) {
        #if os(macOS)
        NSWorkspace.shared.open(url)
        #else
        UIApplication.shared.open(url)
        #endif
    }

    init(resources: @escaping @MainActor (String) -> Data?) {
        self.resources = resources
        let configuration = WKWebViewConfiguration()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = false
        configuration.setURLSchemeHandler(
            ODCResourceSchemeHandler(resources: resources),
            forURLScheme: ODCResource.scheme
        )
        #if os(macOS)
        webView = AppearanceAwareWebView(frame: .zero, configuration: configuration)
        #else
        webView = WKWebView(frame: .zero, configuration: configuration)
        #endif
        super.init()
        webView.navigationDelegate = self
        webView.isInspectable = false
        webView.allowsBackForwardNavigationGestures = false
        #if os(macOS)
        webView.underPageBackgroundColor = .clear
        #else
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        #endif
    }

    /// Loads `html` unless it is already on screen.
    func load(html: String) {
        guard html != loadedHTML else { return }
        loadedHTML = html
        // Inline first: rewriteResources() would turn each <link href> into an
        // odc-resource URL, which no longer names a repository key.
        let inlined = EntryHTML.inlineStylesheets(in: html, resource: resources)
        let body = EntryHTML.rewriteResources(inlined.html)
        if !inlined.skipped.isEmpty {
            renderLog.debug("entry stylesheets skipped: \(inlined.skipped.joined(separator: ", "), privacy: .public)")
        }
        webView.loadHTMLString(
            EntryHTML.document(html: body, stylesheets: inlined.stylesheets),
            baseURL: ODCResource.baseURL
        )
    }

    // MARK: - Navigation

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        renderLog.debug("start \(webView.url?.absoluteString ?? "-", privacy: .public)")
    }

    /// Re-read the document height after the layout settled.
    ///
    /// The height depends on how wide the entry is laid out, so a column resize
    /// has to re-measure; an unchanged width is skipped to keep SwiftUI from
    /// looping on its own height updates.
    func measureContentHeightAfterLayout() {
        Task { @MainActor in
            await Task.yield()
            self.measureContentHeight()
        }
    }

    /// Ask the document how tall it is and hand the answer to `onContentHeight`.
    func measureContentHeight() {
        guard onContentHeight != nil else { return }
        let width = webView.bounds.width
        if let measuredWidth, abs(measuredWidth - width) < 0.5 { return }
        measuredWidth = width
        webView.evaluateJavaScript(
            "(function () { var body = document.body; if (!body) { return 0; } " +
            "var style = getComputedStyle(body); " +
            "return Math.ceil(body.getBoundingClientRect().height " +
            "+ parseFloat(style.marginTop || 0) + parseFloat(style.marginBottom || 0)); })()"
        ) { [weak self] value, error in
            Task { @MainActor in
                guard let self, error == nil, let height = value as? NSNumber else { return }
                let reported = CGFloat(truncating: height)
                guard reported > 0 else { return }
                self.onContentHeight?(reported)
            }
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        renderLog.debug("finish \(webView.url?.absoluteString ?? "-", privacy: .public)")
        // -1 means the document was not a complete page; measuring it would
        // report a blank entry that is only mid-load.
        webView.evaluateJavaScript("document.readyState === 'complete' && document.body ? document.body.innerText.length : -1") { [weak self] value, error in
            Task { @MainActor in
                guard let self else { return }
                if let error {
                    renderLog.error("measure failed: \(error.localizedDescription, privacy: .public)")
                    self.onOutcome?(.failed(error.localizedDescription))
                    return
                }
                let characters = (value as? NSNumber)?.intValue ?? -1
                guard characters >= 0 else { return }
                renderLog.debug("visible characters: \(characters)")
                self.onOutcome?(.loaded(visibleCharacters: characters))
            }
        }
        // A new document invalidates the old width, so measure it afresh.
        measuredWidth = nil
        measureContentHeight()
        Task { @MainActor in
            // Images and late fonts can still grow the document; one more look
            // a moment later keeps the section from clipping them.
            try? await Task.sleep(for: .milliseconds(500))
            self.measuredWidth = nil
            self.measureContentHeight()
        }
    }

    func webView(
        _ webView: WKWebView,
        didFail navigation: WKNavigation!,
        withError error: any Error
    ) {
        reportFailure(error)
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: any Error
    ) {
        reportFailure(error)
    }

    /// A superseded load reports a cancellation error; that is not a failure.
    private func reportFailure(_ error: any Error) {
        guard (error as NSError).code != NSURLErrorCancelled else { return }
        renderLog.error("failed: \(error.localizedDescription, privacy: .public)")
        onOutcome?(.failed(error.localizedDescription))
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        preferences: WKWebpagePreferences,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy, WKWebpagePreferences) -> Void
    ) {
        preferences.allowsContentJavaScript = false
        guard let url = navigationAction.request.url else {
            decisionHandler(.allow, preferences)
            return
        }
        switch url.scheme?.lowercased() {
        case "entry":
            if let target = EntryHTML.entryTarget(from: url) {
                onEntryLink?(target)
            }
            decisionHandler(.cancel, preferences)
        case "http", "https", "mailto":
            if navigationAction.navigationType == .linkActivated {
                openExternally(url)
                decisionHandler(.cancel, preferences)
            } else {
                decisionHandler(.allow, preferences)
            }
        case "odc-resource":
            let key = ODCResource.key(from: url)
            if let key, ODCResource.isAudio(key), let payload = resources(key) {
                audio.play(payload)
                // A tapped speaker icon plays in place instead of navigating.
                decisionHandler(.cancel, preferences)
            } else if navigationAction.navigationType == .linkActivated {
                // A link to a resource (an image, say) must not replace the entry.
                decisionHandler(.cancel, preferences)
            } else {
                // What is left is the document `loadHTMLString` is loading (its
                // base URL carries no key) and the resources it references.
                // Cancelling those leaves the reader on a blank page.
                decisionHandler(.allow, preferences)
            }
        case "about", "data", "blob":
            decisionHandler(.allow, preferences)
        default:
            decisionHandler(.cancel, preferences)
        }
    }
}



/// Plays one MDD audio clip when the reader taps a speaker icon.
///
/// The clips are short pronunciation samples, so a single player per reader
/// is enough: the next clip stops the previous one.
@MainActor
private final class ResourceAudioPlayer {
    private var player: AVAudioPlayer?

    func play(_ data: Data) {
        player?.stop()
        guard let next = try? AVAudioPlayer(data: data) else { return }
        player = next
        next.play()
    }
}

/// Serves MDD payloads to the web view under the `odc-resource` scheme.
///
/// The handler answers on the main actor and reads straight from the open
/// repository; a missing key is a 404, which leaves the entry readable.
@MainActor
final class ODCResourceSchemeHandler: NSObject, WKURLSchemeHandler {
    private let resources: @MainActor (String) -> Data?

    init(resources: @escaping @MainActor (String) -> Data?) {
        self.resources = resources
        super.init()
    }

    func webView(_ webView: WKWebView, start urlSchemeTask: any WKURLSchemeTask) {
        guard let url = urlSchemeTask.request.url else {
            urlSchemeTask.didFailWithError(URLError(.badURL))
            return
        }
        guard let key = ODCResource.key(from: url), let data = resources(key) else {
            let response = HTTPURLResponse(
                url: url,
                statusCode: 404,
                httpVersion: nil,
                headerFields: nil
            )
            if let response {
                urlSchemeTask.didReceive(response)
                urlSchemeTask.didReceive(Data())
                urlSchemeTask.didFinish()
            } else {
                urlSchemeTask.didFailWithError(URLError(.badURL))
            }
            return
        }
        let response = URLResponse(
            url: url,
            mimeType: ODCResource.mimeType(for: key),
            expectedContentLength: data.count,
            textEncodingName: nil
        )
        urlSchemeTask.didReceive(response)
        urlSchemeTask.didReceive(data)
        urlSchemeTask.didFinish()
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: any WKURLSchemeTask) {
        // The payload is handed over synchronously in `start`, so there is
        // nothing to cancel here.
    }
}
#if os(macOS)

/// Keeps the page background transparent when the window changes appearance;
/// the page's own `prefers-color-scheme` media query does the rest.
private final class AppearanceAwareWebView: WKWebView {
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        underPageBackgroundColor = .clear
        needsDisplay = true
    }
}

private struct HTMLRenderRepresentable: NSViewRepresentable {
    let html: String
    let resources: @MainActor @Sendable (String) -> Data?
    let onEntryLink: @MainActor @Sendable (String) -> Void
    let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?
    let onContentHeight: (@MainActor (CGFloat) -> Void)?

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeNSView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateNSView(_ nsView: WKWebView, context: Context) {
        update(context.coordinator)
        context.coordinator.measureContentHeightAfterLayout()
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.onOutcome = onOutcome
        renderer.onContentHeight = onContentHeight
        renderer.load(html: html)
    }
}

#else

private struct HTMLRenderRepresentable: UIViewRepresentable {
    let html: String
    let resources: @MainActor @Sendable (String) -> Data?
    let onEntryLink: @MainActor @Sendable (String) -> Void
    let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?
    let onContentHeight: (@MainActor (CGFloat) -> Void)?

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeUIView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        update(context.coordinator)
        context.coordinator.measureContentHeightAfterLayout()
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.onOutcome = onOutcome
        renderer.onContentHeight = onContentHeight
        renderer.load(html: html)
    }
}

#endif
