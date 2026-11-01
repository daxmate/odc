import SwiftUI

/// Sidebar: the hits of the current query, grouped by dictionary.
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
            stackResults
            emptyResultsRow
        }
        .searchable(text: queryBinding, prompt: "搜索词条")
        .onSubmit(of: .search) { store.submitQuery() }
        .navigationTitle(store.documentName ?? "ODict")
    }

    /// Section title plus one line of context when a dictionary only found
    /// substring matches, so the rows do not look like broken prefix results.
    @ViewBuilder
    private func sectionHeader(for group: DictionarySearchGroup) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(group.name)
            if group.matchKind == .substring {
                Text("没有以「\(trimmedQuery)」开头的词条，以下是包含它的词条")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var trimmedQuery: String {
        store.query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var splitResults: some View {
        ForEach(store.searchGroups) { group in
            if !group.hits.isEmpty {
                Section {
                    ForEach(group.hits) { hit in
                        SearchResultRow(hit: hit)
                            .tag(
                                DictionaryHitSelection(
                                    dictionaryID: group.dictionaryID,
                                    key: hit.key
                                )
                            )
                    }
                } header: {
                    sectionHeader(for: group)
                }
            }
        }
    }

    private var stackResults: some View {
        ForEach(store.searchGroups) { group in
            if !group.hits.isEmpty {
                Section {
                    ForEach(group.hits) { hit in
                        Button {
                            Task { await store.select(key: hit.key, in: group.dictionaryID) }
                        } label: {
                            SearchResultRow(hit: hit)
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(.primary)
                    }
                } header: {
                    sectionHeader(for: group)
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
                        ? "输入以搜索"
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
