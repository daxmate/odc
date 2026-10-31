import SwiftUI

/// Search field plus the prefix-search result list.
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

    private var selectionBinding: Binding<String?> {
        Binding(
            get: { store.selectedKey },
            set: { key in
                guard let key else { return }
                Task { await store.select(key: key) }
            }
        )
    }

    private var listContent: some View {
        ForEach(store.hits) { hit in
            SearchResultRow(hit: hit)
        }
    }

    /// macOS and regular-width iPad: list with a selection that drives the detail column.
    private var splitList: some View {
        List(selection: selectionBinding) {
            listContent
        }
        .overlay {
            if store.hits.isEmpty {
                emptyResults
            }
        }
        .searchable(text: queryBinding, placement: .sidebar, prompt: "搜索词条")
        .navigationTitle("ODict")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
    }

    /// iPhone: value-based links pushed onto the enclosing NavigationStack.
    private var stackList: some View {
        List {
            ForEach(store.hits) { hit in
                NavigationLink(value: hit.key) {
                    SearchResultRow(hit: hit)
                }
            }
        }
        .overlay {
            if store.hits.isEmpty {
                emptyResults
            }
        }
        .searchable(text: queryBinding, prompt: "搜索词条")
        .navigationTitle(store.documentName ?? "ODict")
    }

    @ViewBuilder
    private var emptyResults: some View {
        if store.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            ContentUnavailableView(
                "输入以搜索",
                systemImage: "text.magnifyingglass",
                description: Text("v0 只支持前缀搜索。")
            )
        } else if !store.isLoading {
            ContentUnavailableView.search(text: store.query)
        }
    }
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
