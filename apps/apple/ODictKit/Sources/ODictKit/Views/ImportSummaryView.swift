import SwiftUI

/// Bottom banner that reports a finished MDX conversion until dismissed.
public struct ImportSummaryView: View {
    private let summary: String
    private let dismiss: () -> Void

    public init(summary: String, dismiss: @escaping () -> Void) {
        self.summary = summary
        self.dismiss = dismiss
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.tint)
            Text(summary)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
            Button("好", action: dismiss)
                .buttonStyle(.borderless)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            RoundedRectangle(cornerRadius: 12).strokeBorder(.quaternary)
        }
        .padding(.horizontal, 24)
        .accessibilityElement(children: .combine)
    }
}
