import SwiftUI

/// Shown until a dictionary is opened: an explanation plus an Open button.
public struct EmptyStateView: View {
    @Binding private var isImporterPresented: Bool

    public init(isImporterPresented: Binding<Bool>) {
        _isImporterPresented = isImporterPresented
    }

    public var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "character.book.closed.fill")
                .font(.system(size: 54))
                .foregroundStyle(.tint)
            Text("ODict")
                .font(.largeTitle.weight(.semibold))
            Text("打开一个 .odc 词典，开始只读查词。")
                .foregroundStyle(.secondary)
            Button("打开词典…") {
                isImporterPresented = true
            }
            .buttonStyle(.borderedProminent)
            .padding(.top, 4)
            Text("v0 只读：前缀搜索、词条渲染。编辑与生词本尚未实现。")
                .font(.footnote)
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
