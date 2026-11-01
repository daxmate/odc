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
    private let onEntryLink: @MainActor @Sendable (String) -> Void

    /// - Parameters:
    ///   - html: The entry's raw HTML.
    ///   - onEntryLink: Called with a target key when a link uses `entry://`.
    public init(
        html: String,
        onEntryLink: @escaping @MainActor @Sendable (String) -> Void = { _ in }
    ) {
        self.html = html
        self.onEntryLink = onEntryLink
    }

    public var body: some View {
        HTMLRenderRepresentable(html: html, onEntryLink: onEntryLink)
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

    /// Hands a tapped http(s) link to the system browser.
    private func openExternally(_ url: URL) {
        #if os(macOS)
        NSWorkspace.shared.open(url)
        #else
        UIApplication.shared.open(url)
        #endif
    }

    override init() {
        let configuration = WKWebViewConfiguration()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = false
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
        webView.loadHTMLString(EntryHTML.document(html: html), baseURL: nil)
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
        case "about", "data", "blob":
            decisionHandler(.allow, preferences)
        default:
            decisionHandler(.cancel, preferences)
        }
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
    let onEntryLink: @MainActor @Sendable (String) -> Void

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer()
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
    let onEntryLink: @MainActor @Sendable (String) -> Void

    func makeCoordinator() -> HTMLRenderer {
        HTMLRenderer()
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
