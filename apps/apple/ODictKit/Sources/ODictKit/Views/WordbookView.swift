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

    @AppStorage("odict.wordbook.sort") private var sortRawValue = WordbookSort.recent.rawValue
    @AppStorage("odict.wordbook.filter") private var filterRawValue = WordbookFilter.all.storageValue
    @AppStorage("odict.wordbook.dictionary") private var dictionaryRawValue = ""

    public init(store: DictionaryStore) {
        self.store = store
    }

    public var body: some View {
        NavigationStack {
            List {
                Section {
                    Picker("排序", selection: $sortRawValue) {
                        ForEach(WordbookSort.allCases) { sort in
                            Text(sort.title).tag(sort.rawValue)
                        }
                    }
                    .pickerStyle(.segmented)

                    Picker("筛选", selection: $filterRawValue) {
                        ForEach([WordbookFilter.all, .bookmarks, .history]) { filter in
                            Text(filter.title).tag(filter.storageValue)
                        }
                    }
                    .pickerStyle(.segmented)

                    if !store.library.items.isEmpty {
                        Picker("词典", selection: $dictionaryRawValue) {
                            Text("全部词典").tag("")
                            ForEach(store.library.items) { item in
                                Text(item.displayName).tag(item.id.uuidString)
                            }
                        }
                    }
                }

                Section {
                    if presentation.isEmpty {
                        hint(presentation.emptyMessage)
                    }
                    ForEach(presentation.entries) { entry in
                        entryRow(for: entry)
                    }
                } header: {
                    Text("共 \(presentation.count) 条")
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

    private var presentation: WordbookPresentation {
        WordbookPresentation(
            bookmarks: store.bookmarkedEntries,
            history: store.recentEntries,
            filter: WordbookFilter(storageValue: filterRawValue) ?? .all,
            dictionaryID: UUID(uuidString: dictionaryRawValue),
            sort: WordbookSort(rawValue: sortRawValue) ?? .recent,
            dictionaryNames: dictionaryNames
        )
    }

    private var dictionaryNames: [UUID: String] {
        Dictionary(uniqueKeysWithValues: store.library.items.map { ($0.id, $0.displayName) })
    }

    /// A bookmarked or remembered entry: the headword with its dictionary
    /// named underneath.
    ///
    /// `openEntry` reports failure through the store's messages rather than a
    /// return value, so the sheet always closes; a dictionary that has gone
    /// missing explains itself in the root view.
    private func entryRow(for entry: WordbookEntry) -> some View {
        Button {
            Task {
                await store.openEntry(entry.ref)
                dismiss()
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.ref.headword)
                Text(entry.dictionaryName)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// One grey line standing in for a list that has nothing in it yet.
    private func hint(_ text: String) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
    }
}

#Preview {
    WordbookView(store: DictionaryStore())
}
