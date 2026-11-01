import SwiftUI

/// Sheet listing every imported dictionary, with the explicit cleanup command.
public struct LibraryListView: View {
    private let store: DictionaryStore
    @Environment(\.dismiss) private var dismiss
    @State private var pendingRemoval: LibraryItem?
    @State private var isConfirmingCleanup = false
    @State private var cleanupMessage: String?

    public init(store: DictionaryStore) {
        self.store = store
    }

    public var body: some View {
        NavigationStack {
            content
                .navigationTitle("词典库")
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("完成") { dismiss() }
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            isConfirmingCleanup = true
                        } label: {
                            Label("清理重复副本", systemImage: "arrow.triangle.2.circlepath")
                        }
                        .disabled(store.isDeduplicating)
                    }
                }
        }
        #if os(macOS)
        .frame(minWidth: 440, minHeight: 380)
        #endif
        .confirmationDialog(
            removalTitle,
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { presented in
                    if !presented { pendingRemoval = nil }
                }
            ),
            presenting: pendingRemoval
        ) { item in
            Button("移除并删除本地副本", role: .destructive) {
                store.removeItem(id: item.id)
                pendingRemoval = nil
            }
            Button("取消", role: .cancel) { pendingRemoval = nil }
        } message: { _ in
            Text("本机上的副本会被删除，之后需要重新导入这个词典文件。")
        }
        .confirmationDialog(
            "清理重复副本？",
            isPresented: $isConfirmingCleanup
        ) {
            Button("移动重复副本") {
                Task { await cleanUp() }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("内容相同的副本会被移到词典库目录下的 .Trash 文件夹，可手动恢复。")
        }
        .alert(
            "词典库",
            isPresented: Binding(
                get: { cleanupMessage != nil },
                set: { presented in
                    if !presented { cleanupMessage = nil }
                }
            )
        ) {
            Button("好", role: .cancel) { cleanupMessage = nil }
        } message: {
            Text(cleanupMessage ?? "")
        }
    }

    private var removalTitle: String {
        if let item = pendingRemoval {
            return "移除「\(item.displayName)」？"
        }
        return "移除这本词典？"
    }

    @ViewBuilder
    private var content: some View {
        if store.library.items.isEmpty {
            ContentUnavailableView(
                "词典库为空",
                systemImage: "books.vertical",
                description: Text("导入的词典会留在这里，重启应用后依然可以打开。")
            )
        } else {
            List {
                if !store.migrationIssues.isEmpty {
                    Section("未能迁入词典库") {
                        ForEach(store.migrationIssues) { issue in
                            migrationRow(for: issue)
                        }
                    }
                }
                Section("词典") {
                    if store.library.items.isEmpty {
                        hint("还没有词典")
                    }
                    ForEach(store.library.items) { item in
                        row(for: item)
                    }
                }
                Section("最近") {
                    if store.recentEntries.isEmpty {
                        hint("打开过的词条会出现在这里")
                    }
                    ForEach(store.recentEntries) { ref in
                        memoryRow(for: ref)
                    }
                }
                Section("收藏") {
                    if store.bookmarkedEntries.isEmpty {
                        hint("收藏的词条会出现在这里")
                    }
                    ForEach(store.bookmarkedEntries) { ref in
                        memoryRow(for: ref)
                    }
                }
            }
        }
    }

    /// A remembered entry from the history or bookmark section.
    ///
    /// Tapping it opens the entry; when it belongs to another dictionary the
    /// store switches to that one first.
    private func memoryRow(for ref: EntryRef) -> some View {
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

    /// One grey line standing in for a section that has nothing yet.
    private func hint(_ text: String) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
    }

    /// A dictionary whose copy could not be moved into the library folder.
    ///
    /// The entry is still listed below and still opens from where it lives; the
    /// row only explains why the move did not happen.
    private func migrationRow(for issue: LibraryStore.MigrationIssue) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle")
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 2) {
                Text(issue.displayName)
                Text(issue.reason)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func row(for item: LibraryItem) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "book.closed")
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.displayName)
                    .lineLimit(1)
                Text(detail(for: item))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("打开") {
                store.open(item: item.id)
                dismiss()
            }
            Button(role: .destructive) {
                pendingRemoval = item
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("移除 \(item.displayName)")
        }
        .padding(.vertical, 2)
    }

    private func detail(for item: LibraryItem) -> String {
        if store.documentURL?.lastPathComponent == item.storedName {
            return "已打开"
        }
        return "导入于 \(item.importedAt.formatted(date: .abbreviated, time: .shortened))"
    }

    private func cleanUp() async {
        guard let report = await store.deduplicateExistingCopies() else {
            cleanupMessage = store.errorMessage ?? "清理失败。"
            return
        }
        if report.isEmpty {
            cleanupMessage = "没有发现重复副本。"
        } else {
            let bytes = ByteCountFormatter.string(
                fromByteCount: report.movedBytes,
                countStyle: .file
            )
            let folder = report.trashDirectory?.path ?? ""
            cleanupMessage = "已移出 \(report.removedCount) 份重复副本（\(bytes)），可在 \(folder) 恢复。"
        }
    }
}

#Preview {
    LibraryListView(store: DictionaryStore())
}
