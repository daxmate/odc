import SwiftUI

/// The three first-run buttons, shared by the empty reader and the library
/// sheet so both entry points stay in sync.
public struct LibraryStartActionsView: View {
    private let store: DictionaryStore
    @Binding private var isImporterPresented: Bool
    @State private var isNaming = false
    @State private var draftName = ""

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

            Button {
                isImporterPresented = true
            } label: {
                Label(LibraryStartAction.open.title, systemImage: LibraryStartAction.open.systemImage)
            }
            .buttonStyle(.bordered)
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
