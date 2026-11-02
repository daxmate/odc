#if os(macOS)
import AppKit
import SwiftUI

/// Restores the desktop window's frame and, best effort, the sidebar width.
///
/// SwiftUI's `NavigationSplitView` has no public column-width API, so the
/// width is delegated to the `NSSplitView` AppKit builds underneath it: the
/// probe names that view, which makes AppKit autosave and restore the divider
/// position. If AppKit ever stops backing the split view with an `NSSplitView`
/// the hook simply finds nothing and the window still restores its frame.
public struct DesktopWindowStateModifier: ViewModifier {
    public static let frameAutosaveName = "ODictDesktopMainWindow"
    public static let splitAutosaveName = "ODictDesktopSplitView"

    public init() {}

    public func body(content: Content) -> some View {
        content.background(WindowStateProbe())
    }
}

private struct WindowStateProbe: NSViewRepresentable {
    func makeNSView(context: Context) -> ProbeView {
        ProbeView()
    }

    func updateNSView(_ nsView: ProbeView, context: Context) {
        nsView.applyWindowState()
    }
}

private final class ProbeView: NSView {
    private var didRestoreFrame = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        applyWindowState()
    }

    func applyWindowState() {
        guard let window else { return }
        let name = DesktopWindowStateModifier.frameAutosaveName
        if window.frameAutosaveName != name {
            _ = window.setFrameAutosaveName(name)
        }
        if !didRestoreFrame {
            didRestoreFrame = true
            // SwiftUI may not have applied the saved frame at this point.
            _ = window.setFrameUsingName(name)
        }
        applySplitAutosave(in: window)
    }

    private func applySplitAutosave(in window: NSWindow) {
        guard let splitView = Self.firstSplitView(in: window.contentView) else { return }
        if splitView.autosaveName?.isEmpty ?? true {
            splitView.autosaveName = DesktopWindowStateModifier.splitAutosaveName
        }
    }

    private static func firstSplitView(in view: NSView?) -> NSSplitView? {
        guard let view else { return nil }
        if let splitView = view as? NSSplitView {
            return splitView
        }
        for subview in view.subviews {
            if let found = firstSplitView(in: subview) {
                return found
            }
        }
        return nil
    }
}
#endif
