import AVFoundation
import ODictCore
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
/// log stream --predicate 'subsystem == "com.daxmate.odict" && category == "render"'.
private let renderLog = Logger(subsystem: "com.daxmate.odict", category: "render")

/// Shows the entry page in a web view that cannot run scripts.
///
/// The web view owns the scrolling: it carries an NSScrollView of its own, so a
/// document that is exactly as tall as its content can never hand the wheel to
/// an enclosing SwiftUI scroll view. One page for every dictionary is loaded
/// instead, and the web view scrolls it natively.
public struct HTMLRenderView: View {
    private let document: String
    private let resources: @MainActor @Sendable (Int, String) -> Data?
    private let onEntryLink: @MainActor @Sendable (Int, String) -> Void
    private let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?

    /// - Parameters:
    ///   - document: A complete page, normally built by EntryHTML.page(_:).
    ///   - resources: Payload for one MDD key, nil when the dictionary has
    ///     none. The section names the dictionary that asked for it.
    ///   - onEntryLink: Called with the section and target key of an entry link.
    public init(
        document: String,
        resources: @escaping @MainActor @Sendable (Int, String) -> Data? = { _, _ in nil },
        onEntryLink: @escaping @MainActor @Sendable (Int, String) -> Void = { _, _ in },
        onOutcome: (@MainActor (EntryRenderOutcome) -> Void)? = nil
    ) {
        self.document = document
        self.resources = resources
        self.onEntryLink = onEntryLink
        self.onOutcome = onOutcome
    }

    public var body: some View {
        HTMLRenderRepresentable(
            document: document,
            resources: resources,
            onEntryLink: onEntryLink,
            onOutcome: onOutcome
        )
    }
}

/// Owns the one web view a renderer instance reuses, and routes its link taps.
@MainActor
final class HTMLRenderer: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    var onEntryLink: (@MainActor @Sendable (Int, String) -> Void)?
    /// Reports the document's fate so the reader can replace a blank web view.
    var onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?
    /// Last document handed to the web view; reloading the same HTML would only
    /// make SwiftUI redraws flicker.
    private var loadedDocument: String?
    /// Payload source for odc-resource links and the scheme handler.
    private let resources: @MainActor (Int, String) -> Data?
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

    init(resources: @escaping @MainActor (Int, String) -> Data?) {
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
        #if os(macOS)
        webView.underPageBackgroundColor = .clear
        #else
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        #endif
        webView.isInspectable = false
        webView.allowsBackForwardNavigationGestures = false
        webView.navigationDelegate = self
    }

    /// Loads document unless it is already on screen.
    func load(document: String) {
        guard document != loadedDocument else { return }
        loadedDocument = document
        webView.loadHTMLString(document, baseURL: ODCResource.baseURL)
    }

    // MARK: - Navigation

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        renderLog.debug("start \(webView.url?.absoluteString ?? "-", privacy: .public)")
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        renderLog.debug("finish \(webView.url?.absoluteString ?? "-", privacy: .public)")
        // -1 means the document was not a complete page; counting it would
        // report a blank entry that is only mid-load.
        webView.evaluateJavaScript(Self.visibleTextScript) { [weak self] value, error in
            Task { @MainActor in
                guard let self else { return }
                if let error {
                    renderLog.debug("visible text failed: \(error.localizedDescription, privacy: .public)")
                    self.onOutcome?(.failed(error.localizedDescription))
                    return
                }
                guard let characters = (value as? NSNumber)?.intValue, characters >= 0 else { return }
                self.onOutcome?(.loaded(visibleCharacters: characters))
            }
        }
    }

    /// Counts the text the reader can see.
    ///
    /// body.innerText stops at a shadow boundary, and every section now lives
    /// in one, so the script walks shadow roots (skipping styles and scripts)
    /// and collapses whitespace the way innerText does.
    static let visibleTextScript = """
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
      if (document.readyState !== "complete" || !document.body) { return -1; }
      return text(document.body).replace(/\\s+/g, " ").trim().length;
    })()
    """

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
        renderLog.debug("failed: \(error.localizedDescription, privacy: .public)")
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
            if let addressed = EntryHTML.addressedEntryTarget(from: url) {
                onEntryLink?(addressed.section ?? 0, addressed.target)
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
            if let addressed = ODCResource.address(from: url) {
                if ODCResource.isAudio(addressed.key) {
                    if let data = resources(addressed.section ?? 0, addressed.key) {
                        audio.play(data)
                    }
                    decisionHandler(.cancel, preferences)
                    return
                }
                if navigationAction.navigationType == .linkActivated {
                    // A link to a resource (an image, say) must not replace the entry.
                    decisionHandler(.cancel, preferences)
                    return
                }
            }
            // Images, styles and media load in place; the entry stays put.
            decisionHandler(.allow, preferences)
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

/// Serves MDD payloads to the web view under the odc-resource scheme.
///
/// The first path component names the section that asked for the payload, so a
/// page that stacks several dictionaries can hand each one its own file.
final class ODCResourceSchemeHandler: NSObject, WKURLSchemeHandler {
    private let resources: @MainActor (Int, String) -> Data?

    init(resources: @escaping @MainActor (Int, String) -> Data?) {
        self.resources = resources
        super.init()
    }

    func webView(_ webView: WKWebView, start urlSchemeTask: any WKURLSchemeTask) {
        guard let url = urlSchemeTask.request.url else {
            urlSchemeTask.didFailWithError(URLError(.badURL))
            return
        }
        guard let addressed = ODCResource.address(from: url),
              let data = resources(addressed.section ?? 0, addressed.key)
        else {
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
        let key = addressed.key
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
        // The payload is handed over synchronously in start, so there is
        // nothing to cancel here.
    }
}

#if os(macOS)

/// Keeps the page background transparent when the system appearance changes:
/// WebKit resets underPageBackgroundColor on a dynamic switch.
private final class AppearanceAwareWebView: WKWebView {
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        underPageBackgroundColor = .clear
        needsDisplay = true
    }
}

private struct HTMLRenderRepresentable: NSViewRepresentable {
    let document: String
    let resources: @MainActor @Sendable (Int, String) -> Data?
    let onEntryLink: @MainActor @Sendable (Int, String) -> Void
    let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeNSView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateNSView(_ webView: WKWebView, context: Context) {
        update(context.coordinator)
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.onOutcome = onOutcome
        renderer.load(document: document)
    }
}

#else

private struct HTMLRenderRepresentable: UIViewRepresentable {
    let document: String
    let resources: @MainActor @Sendable (Int, String) -> Data?
    let onEntryLink: @MainActor @Sendable (Int, String) -> Void
    let onOutcome: (@MainActor (EntryRenderOutcome) -> Void)?

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeUIView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        update(context.coordinator)
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.onOutcome = onOutcome
        renderer.load(document: document)
    }
}

#endif
