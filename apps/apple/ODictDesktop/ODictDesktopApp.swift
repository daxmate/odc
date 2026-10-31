import ODictKit
import SwiftUI

@main
struct ODictDesktopApp: App {
    @State private var store = DictionaryStore()
    @State private var isImporterPresented = false

    var body: some Scene {
        WindowGroup {
            DictionaryRootView(store: store, isImporterPresented: $isImporterPresented)
                #if os(macOS)
                .frame(minWidth: 720, minHeight: 480)
                #endif
        }
        .defaultSize(width: 900, height: 620)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Open…") {
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
