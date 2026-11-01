import SwiftUI

/// Root scene shared by the iOS and macOS targets.
public struct DictionaryRootView: View {
    private let store: DictionaryStore
    @Binding private var isImporterPresented: Bool
    @State private var isLibraryPresented = false
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
                    openDocument(url)
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
            .overlay(alignment: .bottom) {
                if let phase = store.importPhase {
                    ImportProgressView(phase: phase)
                        .padding(.bottom, 24)
                        .transition(.opacity)
                } else if let summary = store.importSummary {
                    ImportSummaryView(summary: summary) {
                        store.dismissImportSummary()
                    }
                    .padding(.bottom, 24)
                }
            }
            .odictOnOpenURL { url in
                openDocument(url)
            }
            .sheet(isPresented: $isLibraryPresented) {
                LibraryListView(store: store)
            }
    }

    /// MDX files are converted into a standalone document first.
    private func openDocument(_ url: URL) {
        store.openDocument(url: url)
    }

    @ViewBuilder
    private var content: some View {
        if store.documentURL == nil {
            EmptyStateView(store: store, isImporterPresented: $isImporterPresented)
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
        ToolbarItemGroup(placement: .primaryAction) {
            Button {
                isLibraryPresented = true
            } label: {
                Label("词典库", systemImage: "books.vertical")
            }
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
