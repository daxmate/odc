import AppKit
import ODictKit
import SwiftUI

@main
struct ODictDesktopApp: App {
    @NSApplicationDelegateAdaptor(ODictDesktopAppDelegate.self) private var appDelegate
    @State private var store = DictionaryStore()
    @State private var isImporterPresented = false

    var body: some Scene {
        WindowGroup {
            DictionaryRootView(store: store, isImporterPresented: $isImporterPresented)
                .frame(minWidth: 720, minHeight: 480)
                .onAppear {
                    appDelegate.connect { urls in
                        for url in urls {
                            store.openDocument(url: url)
                        }
                    }
                }
        }
        .defaultSize(width: 900, height: 620)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Open Dictionary or MDX…") {
                    isImporterPresented = true
                }
                .keyboardShortcut("o", modifiers: .command)
            }
            CommandGroup(after: .saveItem) {
                Button("Close Dictionary") {
                    store.close()
                }
                .keyboardShortcut("w", modifiers: [.command, .shift])
                .disabled(store.documentURL == nil)
            }
        }
    }
}

/// Receives Finder "Open With" events.
///
/// SwiftUI does not deliver those reliably on macOS, so the app uses an
/// `NSApplicationDelegate` and forwards the URLs to the store. Events that
/// arrive before the window exists are buffered and flushed by `connect`.
@MainActor
final class ODictDesktopAppDelegate: NSObject, NSApplicationDelegate {
    private var openHandler: (([URL]) -> Void)?
    private var pendingURLs: [URL] = []

    func connect(_ handler: @escaping ([URL]) -> Void) {
        openHandler = handler
        guard !pendingURLs.isEmpty else { return }
        let urls = pendingURLs
        pendingURLs.removeAll()
        handler(urls)
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        if let openHandler {
            openHandler(urls)
        } else {
            pendingURLs.append(contentsOf: urls)
        }
    }
}
