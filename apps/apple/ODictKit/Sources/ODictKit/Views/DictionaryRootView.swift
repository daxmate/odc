import SwiftUI

/// Root scene shared by the iOS and macOS targets.
public struct DictionaryRootView: View {
    private let store: DictionaryStore
    @Binding private var isImporterPresented: Bool
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    public init(store: DictionaryStore, isImporterPresented: Binding<Bool>) {
        self.store = store
        _isImporterPresented = isImporterPresented
    }

    public var body: some View {
        content
            .fileImporter(
                isPresented: $isImporterPresented,
                allowedContentTypes: ODictDocument.readableContentTypes,
                allowsMultipleSelection: false
            ) { result in
                switch result {
                case .success(let urls):
                    guard let url = urls.first else { return }
                    store.open(url: url)
                case .failure(let error):
                    store.present(error: error)
                }
            }
            .alert(
                "词典错误",
                isPresented: Binding(
                    get: { store.errorMessage != nil },
                    set: { presented in
                        if !presented {
                            store.dismissError()
                        }
                    }
                )
            ) {
                Button("好", role: .cancel) {
                    store.dismissError()
                }
            } message: {
                Text(store.errorMessage ?? "")
            }
            .odictOnOpenURL { url in
                store.open(url: url)
            }
    }

    @ViewBuilder
    private var content: some View {
        if store.documentURL == nil {
            EmptyStateView(isImporterPresented: $isImporterPresented)
        } else {
            loadedContent
        }
    }

    #if os(macOS)
    private var loadedContent: some View {
        NavigationSplitView {
            SearchResultsView(store: store)
                .navigationSplitViewColumnWidth(min: 220, ideal: 260)
        } detail: {
            EntryDetailView(store: store)
        }
        .toolbar { openButton }
    }
    #else
    @ViewBuilder
    private var loadedContent: some View {
        if horizontalSizeClass == .regular {
            NavigationSplitView {
                SearchResultsView(store: store)
            } detail: {
                EntryDetailView(store: store)
            }
            .toolbar { openButton }
        } else {
            NavigationStack {
                SearchResultsView(store: store)
                    .navigationDestination(for: String.self) { key in
                        EntryDetailView(store: store, key: key)
                    }
            }
            .toolbar { openButton }
        }
    }
    #endif

    private var openButton: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button {
                isImporterPresented = true
            } label: {
                Label("打开词典", systemImage: "folder")
            }
        }
    }
}

#Preview {
    DictionaryRootView(store: DictionaryStore(), isImporterPresented: .constant(false))
}
