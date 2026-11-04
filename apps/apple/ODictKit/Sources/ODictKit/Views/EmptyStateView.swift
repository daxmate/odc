import SwiftUI

/// Shown until a dictionary is opened: the library (when it has entries)
/// plus an Open button.
public struct EmptyStateView: View {
    private let store: DictionaryStore
    @Binding private var isImporterPresented: Bool

    public init(store: DictionaryStore, isImporterPresented: Binding<Bool>) {
        self.store = store
        _isImporterPresented = isImporterPresented
    }

    public var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "character.book.closed.fill")
                .font(.system(size: 54))
                .foregroundStyle(.tint)
            Text("ODict")
                .font(.largeTitle.weight(.semibold))
            if store.library.items.isEmpty {
                Text(explanation)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                Text("最近词典")
                    .font(.headline)
                VStack(spacing: 6) {
                    ForEach(store.library.items) { item in
                        Button {
                            store.open(item: item.id)
                        } label: {
                            HStack(spacing: 8) {
                                Image(systemName: "book.closed")
                                Text(item.displayName)
                                    .lineLimit(1)
                                Spacer(minLength: 0)
                            }
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .frame(maxWidth: 320)
            }
            LibraryStartActionsView(store: store, isImporterPresented: $isImporterPresented)
                .padding(.top, 4)
            Text("词典会复制到本机，之后可以离线打开。")
                .font(.footnote)
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var explanation: String {
        #if os(macOS)
        return "打开一个 .odc 词典，或导入 .mdx 文件自动转换成 .odc。"
        #else
        return "打开一个 .odc 词典，开始只读查词。"
        #endif
    }
}
