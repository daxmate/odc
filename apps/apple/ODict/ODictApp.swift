import ODictKit
import SwiftUI

@main
struct ODictApp: App {
    @State private var store = DictionaryStore()
    @State private var isImporterPresented = false

    var body: some Scene {
        WindowGroup {
            DictionaryRootView(store: store, isImporterPresented: $isImporterPresented)
        }
    }
}
