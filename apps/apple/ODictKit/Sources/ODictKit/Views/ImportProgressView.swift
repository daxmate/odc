import SwiftUI

/// Bottom banner shown while an MDX conversion runs.
///
/// The bar is determinate as soon as the core knows how big the job is;
/// until then the banner falls back to a spinner and a stage name rather
/// than inventing a percentage.
public struct ImportProgressView: View {
    private let progress: MDXImportProgress

    public init(progress: MDXImportProgress) {
        self.progress = progress
    }

    public var body: some View {
        HStack(spacing: 10) {
            if progress.isDeterminate {
                ProgressView(value: progress.fraction)
                    .progressViewStyle(.linear)
                    .frame(width: 110)
            } else {
                ProgressView()
                    .controlSize(.small)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(.callout)
                if !progress.detail.isEmpty {
                    Text(progress.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .overlay {
            Capsule().strokeBorder(.quaternary)
        }
        .accessibilityElement(children: .combine)
    }

    private var title: String {
        guard progress.isDeterminate else { return progress.title }
        return "\(progress.title) · \(progress.percent)%"
    }
}
