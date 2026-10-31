import SwiftUI

extension View {
    /// Handles document URLs delivered by the system (double-click, "Open in",
    /// drag onto the app).
    ///
    /// iOS routes them through SwiftUI's `onOpenURL`. macOS receives them in
    /// `NSApplicationDelegate.application(_:open:)` instead, because SwiftUI
    /// does not reliably deliver Finder events there.
    @ViewBuilder
    func odictOnOpenURL(perform action: @escaping (URL) -> Void) -> some View {
        #if os(iOS)
        self.onOpenURL(perform: action)
        #else
        self
        #endif
    }
}
