import SwiftUI

/// Sheet listing the words the reader bookmarked or looked up.
///
/// The data already lives in `reader.json`; this view only presents it. The
/// rows mirror the ones the library sheet used to carry: tapping a row opens
/// the entry, switching dictionaries first when the entry belongs to another
/// one, and closes the sheet.
public struct WordbookView: View {
    private let store: DictionaryStore
    @Environment(\.dismiss) private var dismiss

    public init(store: DictionaryStore) {
        self.store = store
    }

    public var body: some View {
        NavigationStack {
            List {
                Section("收藏") {
                    if store.bookmarkedEntries.isEmpty {
                        hint("还没有收藏")
                    }
                    ForEach(store.bookmarkedEntries) { ref in
                        entryRow(for: ref)
                    }
                }
                Section("最近") {
                    if store.recentEntries.isEmpty {
                        hint("还没有查询记录")
                    }
                    ForEach(store.recentEntries) { ref in
                        entryRow(for: ref)
                    }
                }
            }
            .navigationTitle("生词本")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("完成") { dismiss() }
                }
            }
        }
        #if os(macOS)
        .frame(minWidth: 440, minHeight: 380)
        #endif
    }

    /// A bookmarked or remembered entry: the headword with its dictionary
    /// named underneath.
    ///
    /// `openEntry` reports failure through the store's messages rather than a
    /// return value, so the sheet always closes; a dictionary that has gone
    /// missing explains itself in the root view.
    private func entryRow(for ref: EntryRef) -> some View {
        Button {
            Task {
                await store.openEntry(ref)
                dismiss()
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(ref.headword)
                Text(dictionaryName(for: ref))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// Name of the dictionary a remembered entry belongs to.
    private func dictionaryName(for ref: EntryRef) -> String {
        guard let id = UUID(uuidString: ref.dictionaryID) else { return ref.dictionaryID }
        return store.library.item(id: id)?.displayName ?? "已移除的词典"
    }

    /// One grey line standing in for a section that has nothing in it yet.
    private func hint(_ text: String) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
    }
}

#Preview {
    WordbookView(store: DictionaryStore())
}
