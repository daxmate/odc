import AVFoundation
import SwiftUI
import WebKit

#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Shows an entry's HTML exactly as the dictionary wrote it, in a web view that
/// cannot run scripts.
public struct HTMLRenderView: View {
    private let html: String
    private let resources: @MainActor @Sendable (String) -> Data?
    private let onEntryLink: @MainActor @Sendable (String) -> Void

    /// - Parameters:
    ///   - html: The entry's raw HTML.
    ///   - resources: Payload for one MDD key, `nil` when the dictionary has
    ///     none. Served under the `odc-resource` scheme.
    ///   - onEntryLink: Called with a target key when a link uses `entry://`.
    public init(
        html: String,
        resources: @escaping @MainActor @Sendable (String) -> Data? = { _ in nil },
        onEntryLink: @escaping @MainActor @Sendable (String) -> Void = { _ in }
    ) {
        self.html = html
        self.resources = resources
        self.onEntryLink = onEntryLink
    }

    public var body: some View {
        HTMLRenderRepresentable(
            html: html,
            resources: resources,
            onEntryLink: onEntryLink
        )
    }
}

/// Owns the one web view a renderer instance reuses, and routes its link taps.
@MainActor
final class HTMLRenderer: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    var onEntryLink: (@MainActor @Sendable (String) -> Void)?
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
        webView.loadHTMLString(
            EntryHTML.document(html: EntryHTML.rewriteResources(html)),
            baseURL: ODCResource.baseURL
        )
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
            if let key = ODCResource.key(from: url),
               ODCResource.isAudio(key),
               let payload = resources(key) {
                audio.play(payload)
            }
            decisionHandler(.cancel, preferences)
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

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeNSView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateNSView(_ nsView: WKWebView, context: Context) {
        update(context.coordinator)
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.load(html: html)
    }
}

#else

private struct HTMLRenderRepresentable: UIViewRepresentable {
    let html: String
    let resources: @MainActor @Sendable (String) -> Data?
    let onEntryLink: @MainActor @Sendable (String) -> Void

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer(resources: resources)
    }

    func makeUIView(context: Context) -> WKWebView {
        update(context.coordinator)
        return context.coordinator.webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        update(context.coordinator)
    }

    private func update(_ renderer: HTMLRenderer) {
        renderer.onEntryLink = onEntryLink
        renderer.load(html: html)
    }
}

#endif
