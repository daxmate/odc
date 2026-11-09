import SwiftUI
import UniformTypeIdentifiers

/// The three first-run buttons, shared by the empty reader and the library
/// sheet so both entry points stay in sync.
public struct LibraryStartActionsView: View {
    private let store: DictionaryStore
    @Binding private var isImporterPresented: Bool
    @State private var isNaming = false
    @State private var draftName = ""
    @State private var isPickingProfile = false

    public init(store: DictionaryStore, isImporterPresented: Binding<Bool>) {
        self.store = store
        _isImporterPresented = isImporterPresented
    }

    public var body: some View {
        VStack(spacing: 10) {
            Button {
                draftName = ""
                isNaming = true
            } label: {
                Label(LibraryStartAction.newDictionary.title, systemImage: LibraryStartAction.newDictionary.systemImage)
            }
            .buttonStyle(.borderedProminent)

            Button {
                store.createSampleDictionary()
            } label: {
                Label(LibraryStartAction.sample.title, systemImage: LibraryStartAction.sample.systemImage)
            }
            .buttonStyle(.bordered)

            Menu {
                Button {
                    isImporterPresented = true
                } label: {
                    Label("导入 MDX…", systemImage: "doc.badge.plus")
                }
                Button {
                    isPickingProfile = true
                } label: {
                    Label("用适配 profile 导入…", systemImage: "wand.and.stars")
                }
            } label: {
                Label(LibraryStartAction.open.title, systemImage: LibraryStartAction.open.systemImage)
            }
            .buttonStyle(.bordered)
            .accessibilityLabel("打开词典，可选择适配 profile 导入")
        }
        .fileImporter(
            isPresented: $isPickingProfile,
            allowedContentTypes: [.json],
            allowsMultipleSelection: false
        ) { result in
            switch result {
            case .success(let urls):
                guard let url = urls.first else { return }
                // The profile is consumed by the MDX import that starts next;
                // the store clears it as soon as that import begins.
                store.pendingImportProfileURL = url
                isImporterPresented = true
            case .failure(let error):
                store.present(error: error)
            }
        }
        .alert("新建词典", isPresented: $isNaming) {
            TextField("词典名称", text: $draftName)
            Button("创建") {
                store.createDictionaryReportingErrors(named: draftName)
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("会创建一本空词典，之后可以逐条添加词条。")
        }
    }
}
