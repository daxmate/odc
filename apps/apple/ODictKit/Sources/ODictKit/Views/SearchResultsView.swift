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
            if isQueryEmpty {
                recentSearches
            } else {
                splitResults
                emptyResultsRow
            }
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
            if isQueryEmpty {
                recentSearches
            } else {
                stackResults
                emptyResultsRow
            }
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

    /// An empty field swaps the results for the recent searches.
    private var isQueryEmpty: Bool { trimmedQuery.isEmpty }

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
                Text("没有匹配的词条")
                .font(.callout)
                .foregroundStyle(.secondary)
            }
        }
    }
    /// Recent queries; empty until a search is submitted or a result opened.
    @ViewBuilder
    private var recentSearches: some View {
        if !store.searchHistory.items.isEmpty {
            recentSearchesHeader
            ForEach(store.searchHistory.items, id: \.self) { term in
                recentSearchRow(term)
            }
        }
    }

    private var recentSearchesHeader: some View {
        HStack(spacing: 8) {
            Text("最近搜索")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Button("清除") {
                store.searchHistory.clear()
            }
            .font(.footnote)
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
    }

    private func recentSearchRow(_ term: String) -> some View {
        Button {
            store.setQuery(term)
        } label: {
            Label(term, systemImage: "clock")
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        #if os(macOS)
        .contextMenu {
            Button("从历史中删除") {
                store.searchHistory.remove(term)
            }
        }
        #else
        .swipeActions(edge: .trailing) {
            Button(role: .destructive) {
                store.searchHistory.remove(term)
            } label: {
                Label("删除", systemImage: "trash")
            }
        }
        #endif
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
