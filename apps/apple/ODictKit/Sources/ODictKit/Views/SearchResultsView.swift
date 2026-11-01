import SwiftUI

/// Sidebar: the dictionaries that are open and the hits of the current query,
/// grouped by dictionary.
public struct SearchResultsView: View {
    private let store: DictionaryStore
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    public init(store: DictionaryStore) {
        self.store = store
    }

    public var body: some View {
        #if os(macOS)
        splitList
        #else
        if horizontalSizeClass == .regular {
            splitList
        } else {
            stackList
        }
        #endif
    }

    private var queryBinding: Binding<String> {
        Binding(
            get: { store.query },
            set: { store.setQuery($0) }
        )
    }

    /// A row is a (dictionary, key) pair: the same headword can come from two
    /// open dictionaries, and the pair says which one to switch to.
    private var hitSelection: Binding<DictionaryHitSelection?> {
        Binding(
            get: {
                guard let dictionaryID = store.activeDictionaryID,
                      let key = store.selectedKey
                else { return nil }
                return DictionaryHitSelection(dictionaryID: dictionaryID, key: key)
            },
            set: { selection in
                guard let selection else { return }
                Task { await store.select(key: selection.key, in: selection.dictionaryID) }
            }
        )
    }

    private var splitList: some View {
        List(selection: hitSelection) {
            dictionarySection
            splitResults
            emptyResultsRow
        }
        .searchable(text: queryBinding, placement: .sidebar, prompt: "搜索词条")
        .onSubmit(of: .search) { store.submitQuery() }
        .navigationTitle("ODict")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #else
        .modifier(SearchFieldSubmitModifier(submit: { store.submitQuery() }))
        #endif
    }

    private var stackList: some View {
        List {
            dictionarySection
            stackResults
            emptyResultsRow
        }
        .searchable(text: queryBinding, prompt: "搜索词条")
        .onSubmit(of: .search) { store.submitQuery() }
        .navigationTitle(store.documentName ?? "ODict")
    }

    /// Every dictionary in the library, with the open ones marked.
    ///
    /// Tapping a row opens a closed dictionary or switches to an open one; the
    /// trailing button closes it without forgetting the copy.
    private var dictionarySection: some View {
        Section("词典") {
            if store.library.items.isEmpty {
                Text("还没有词典")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            ForEach(store.library.items) { item in
                dictionaryRow(for: item)
            }
        }
    }

    private func dictionaryRow(for item: LibraryItem) -> some View {
        let isOpen = store.isOpen(item.id)
        let isActive = store.isActive(item.id)
        return HStack(spacing: 8) {
            Button {
                openOrActivate(item)
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: isOpen ? "book.fill" : "book.closed")
                        .foregroundStyle(isActive ? Color.accentColor : Color.secondary)
                    Text(item.displayName)
                        .lineLimit(1)
                        .fontWeight(isActive ? .semibold : .regular)
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(
                isOpen
                    ? "切换到 \(item.displayName)"
                    : "打开 \(item.displayName)"
            )
            if isOpen {
                Button {
                    store.closeLibraryItem(id: item.id)
                } label: {
                    Image(systemName: "xmark.circle")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("关闭 \(item.displayName)")
            }
        }
        .listRowBackground(isActive ? Color.accentColor.opacity(0.12) : Color.clear)
    }

    private func openOrActivate(_ item: LibraryItem) {
        if store.isOpen(item.id) {
            store.activateDictionary(id: item.id)
        } else {
            store.open(item: item.id)
        }
    }

    private var splitResults: some View {
        ForEach(store.searchGroups) { group in
            if !group.hits.isEmpty {
                Section(group.name) {
                    ForEach(group.hits) { hit in
                        SearchResultRow(hit: hit)
                            .tag(
                                DictionaryHitSelection(
                                    dictionaryID: group.dictionaryID,
                                    key: hit.key
                                )
                            )
                    }
                }
            }
        }
    }

    private var stackResults: some View {
        ForEach(store.searchGroups) { group in
            if !group.hits.isEmpty {
                Section(group.name) {
                    ForEach(group.hits) { hit in
                        Button {
                            Task { await store.select(key: hit.key, in: group.dictionaryID) }
                        } label: {
                            SearchResultRow(hit: hit)
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(.primary)
                    }
                }
            }
        }
    }

    /// Kept inside the list so the dictionary section above stays clickable.
    @ViewBuilder
    private var emptyResultsRow: some View {
        if !store.isLoading, store.searchGroups.allSatisfy({ $0.hits.isEmpty }) {
            Section {
                Text(
                    store.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        ? "输入以搜索（v0 只支持前缀搜索）"
                        : "没有匹配的词条"
                )
                .font(.callout)
                .foregroundStyle(.secondary)
            }
        }
    }
}

/// One row of the grouped results: which dictionary it came from and its key.
struct DictionaryHitSelection: Hashable {
    let dictionaryID: UUID
    let key: String
}

struct SearchResultRow: View {
    let hit: SearchHit

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(hit.display)
                .font(.body)
            if hit.key != hit.display {
                Text(hit.key)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}
