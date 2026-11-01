#if os(macOS)
import AppKit
import SwiftUI

/// Whether a responder is the sidebar search field's editor.
///
/// SwiftUI on macOS does not deliver Return from `.searchable(placement:
/// .sidebar)` to `.onSubmit(of: .search)`, so the list installs a local key
/// monitor. It must leave every other text field alone, which is what this
/// check decides: only the field editor of an `NSSearchField` qualifies.
@MainActor
enum SearchFieldResponder {
    static func isSearchFieldEditor(_ responder: NSResponder?) -> Bool {
        guard let editor = responder as? NSTextView else { return false }
        // The field editor is a shared NSTextView whose delegate is the
        // control that owns it.
        guard let field = editor.delegate as? NSTextField else { return false }
        // SwiftUI builds the toolbar field as an NSSearchField; a control that
        // only carries a search cell is close enough to be the same box.
        return field is NSSearchField || field.cell is NSSearchFieldCell
    }

    /// True when Return should open the first hit.
    ///
    /// A field that is mid-composition (Chinese, Japanese, Korean input) keeps
    /// the key: there Return accepts the candidate, not the query.
    static func shouldSubmit(responder: NSResponder?) -> Bool {
        guard let editor = responder as? NSTextView, !editor.hasMarkedText() else { return false }
        return isSearchFieldEditor(editor)
    }
}

/// Opens the first hit when Return is pressed in the sidebar search field.
///
/// Installed while the list is on screen and removed on disappear, so a closed
/// document leaves no monitor behind.
struct SearchFieldSubmitModifier: ViewModifier {
    let submit: @MainActor @Sendable () -> Void

    @State private var monitor: Any?

    func body(content: Content) -> some View {
        content
            .onAppear { install() }
            .onDisappear { remove() }
    }

    /// Idempotent: a sidebar rebuilt by the split view keeps one monitor.
    @MainActor
    private func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            // Return, and the numeric keypad's Enter.
            guard event.keyCode == 36 || event.keyCode == 76 else { return event }
            let belongsToSearchField = MainActor.assumeIsolated {
                event.window === NSApp.keyWindow
                    && SearchFieldResponder.shouldSubmit(responder: event.window?.firstResponder)
            }
            guard belongsToSearchField else { return event }
            MainActor.assumeIsolated { submit() }
            // Consumed: the field editor would otherwise beep on Return.
            return nil
        }
    }

    @MainActor
    private func remove() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
        }
        monitor = nil
    }
}
#endif
