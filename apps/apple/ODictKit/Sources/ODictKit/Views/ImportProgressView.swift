import SwiftUI

/// Bottom banner shown while an MDX conversion runs.
///
/// The progress is indeterminate on purpose: the core reports stages, not
/// percentages, so the UI must not invent one.
public struct ImportProgressView: View {
    private let phase: MDXImportPhase

    public init(phase: MDXImportPhase) {
        self.phase = phase
    }

    public var body: some View {
        HStack(spacing: 10) {
            ProgressView()
                .controlSize(.small)
            Text(phase.title)
                .font(.callout)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .overlay {
            Capsule().strokeBorder(.quaternary)
        }
        .accessibilityElement(children: .combine)
    }
}
