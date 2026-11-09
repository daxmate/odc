import SwiftUI

/// Bottom banner that reports a finished MDX conversion until dismissed.
public struct ImportSummaryView: View {
    private let summary: String
    private let isWarning: Bool
    private let dismiss: () -> Void

    /// `isWarning` swaps the checkmark for an orange warning triangle: the
    /// import succeeded, but the profile covered less than expected.
    public init(summary: String, isWarning: Bool = false, dismiss: @escaping () -> Void) {
        self.summary = summary
        self.isWarning = isWarning
        self.dismiss = dismiss
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(
                systemName: isWarning
                    ? "exclamationmark.triangle.fill"
                    : "checkmark.circle.fill"
            )
            .foregroundStyle(isWarning ? AnyShapeStyle(Color.orange) : AnyShapeStyle(.tint))
            .accessibilityHidden(true)
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
        .accessibilityLabel(isWarning ? "导入提醒：\(summary)" : "导入完成：\(summary)")
    }
}
