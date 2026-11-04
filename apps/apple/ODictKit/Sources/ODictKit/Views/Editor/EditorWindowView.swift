#if os(macOS)
import AppKit
import ODictCore
import SwiftUI

/// Separate window hosting the editing surface. The root window owns the open
/// dictionaries; this view only borrows their editing handle.
public struct EditorWindowView: View {
    private let store: DictionaryStore
    @State private var model: DictionaryEditorModel?
    @Environment(\.dismissWindow) private var dismissWindow

    public init(store: DictionaryStore) {
        self.store = store
    }

    public var body: some View {
        Group {
            if store.documentURL == nil {
                ContentUnavailableView(
                    "没有打开的词典",
                    systemImage: "book.closed",
                    description: Text("先在主窗口打开一个词典目录，再进入编辑模式。")
                )
            } else if let model {
                EditorView(
                    model: model,
                    repoPath: store.documentURL?.path,
                    libraryKey: store.activeDictionaryID?.uuidString ?? ""
                ) {
                    dismissWindow(id: "editor")
                }
            } else {
                ContentUnavailableView(
                    "这个词典不能编辑",
                    systemImage: "lock",
                    description: Text("单文件 .odc 词典是只读的；请打开包含 odc.db 的文件夹。")
                )
            }
        }
        .frame(minWidth: 980, minHeight: 600)
        .task(id: store.activeDictionaryID) {
            model = store.makeEditorModel()
            model?.loadIfNeeded()
        }
    }
}

struct EditorView: View {
    @Bindable var model: DictionaryEditorModel
    /// Repository directory of the active dictionary.
    let repoPath: String?
    /// Stable identifier used for the per-library publish record.
    let libraryKey: String
    let onCommitted: () -> Void

    @State private var isCommitPresented = false
    @State private var isImportPresented = false
    @State private var isDiscardPresented = false
    @State private var isDeletePresented = false
    @State private var importer: CSVImportModel?
    @State private var isPublishPresented = false
    @State private var isTrustPresented = false
    @State private var isSettingsPresented = false
    @State private var openSettingsAfterPublish = false
    @State private var publishModel: PublishModel?

    var body: some View {
        VStack(spacing: 0) {
            HSplitView {
                entryList
                    .frame(minWidth: 300, idealWidth: 340, maxWidth: 520)
                detail
                    .frame(minWidth: 560)
            }
            Divider()
            statusBar
        }
        .toolbar {
            ToolbarItemGroup {
                Button {
                    _ = model.createEntry()
                } label: {
                    Label("新建词条", systemImage: "plus")
                }
                .keyboardShortcut("n", modifiers: .command)
                .help("在工作区新建一个词条")
                .disabled(!model.isLoaded)

                Button {
                    isDeletePresented = true
                } label: {
                    Label("删除词条", systemImage: "trash")
                }
                .help("从工作区删除选中的词条")
                .disabled(model.selectedKey == nil)

                Button {
                    _ = model.undo()
                } label: {
                    Label("撤销", systemImage: "arrow.uturn.backward")
                }
                .keyboardShortcut("z", modifiers: .command)
                .help("撤销上一步编辑")
                .disabled(!model.canUndo)

                Button {
                    _ = model.redo()
                } label: {
                    Label("重做", systemImage: "arrow.uturn.forward")
                }
                .keyboardShortcut("z", modifiers: [.command, .shift])
                .help("重做上一步撤销")
                .disabled(!model.canRedo)

                Button {
                    importer = model.makeCSVImportModel()
                    isImportPresented = importer != nil
                } label: {
                    Label("导入 CSV…", systemImage: "tablecells")
                }
                .help("把 CSV 行写入工作区")

                Button {
                    _ = model.save()
                } label: {
                    Label("保存", systemImage: "square.and.arrow.down")
                }
                .keyboardShortcut("s", modifiers: .command)
                .disabled(model.form == nil)

                Button {
                    presentPublish()
                } label: {
                    Label("发布…", systemImage: "shippingbox")
                }
                .help("把 HEAD 打包成已签名发布")
                .disabled(repoPath == nil)

                Button {
                    isTrustPresented = true
                } label: {
                    Label("信任…", systemImage: "checkmark.seal")
                }
                .help("管理这个词典信任的发布者")
                .disabled(repoPath == nil)
            }
        }
        .sheet(isPresented: $isCommitPresented) {
            CommitSheet(model: model) {
                onCommitted()
            }
        }
        .sheet(isPresented: $isImportPresented, onDismiss: {
            importer = nil
            model.reload()
        }) {
            if let importer {
                CSVImportSheet(model: importer) {
                    model.reload()
                }
            }
        }
        .sheet(isPresented: $isPublishPresented, onDismiss: {
            // Presenting one sheet while another dismisses is unreliable on
            // macOS, so the settings sheet waits for this one to close.
            if openSettingsAfterPublish {
                openSettingsAfterPublish = false
                isSettingsPresented = true
            }
        }) {
            if let publishModel {
                PublishSheet(model: publishModel) {
                    openSettingsAfterPublish = true
                }
            }
        }
        .sheet(isPresented: $isTrustPresented) {
            if let repoPath {
                TrustListView(repoPath: repoPath)
            }
        }
        .sheet(isPresented: $isSettingsPresented) {
            PublisherSettingsView()
                .frame(minWidth: 480, minHeight: 440)
        }
        .confirmationDialog(
            "丢弃工作区改动？",
            isPresented: $isDiscardPresented,
            titleVisibility: .visible
        ) {
            Button("丢弃改动", role: .destructive) {
                _ = model.discard()
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("所有未提交的新增、修改和删除都会丢失，无法恢复。")
        }
        .confirmationDialog(
            "删除「\(model.selectedKey ?? "")」？",
            isPresented: $isDeletePresented,
            titleVisibility: .visible
        ) {
            Button("删除", role: .destructive) {
                _ = model.deleteEntry()
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("删除会在工作区标记，提交后才从 HEAD 移除。")
        }
        .alert(
            "无法完成操作",
            isPresented: Binding(
                get: { model.alertMessage != nil },
                set: { if !$0 { model.clearAlert() } }
            )
        ) {
            Button("好", role: .cancel) { model.clearAlert() }
        } message: {
            Text(model.alertMessage ?? "")
        }
    }

    // MARK: - Publishing

    /// Build a fresh publish model for the active repository.
    private func presentPublish() {
        guard let repoPath else { return }
        publishModel = PublishModel(
            repoPath: repoPath,
            libraryKey: libraryKey,
            libraryName: (repoPath as NSString).lastPathComponent
        )
        isPublishPresented = true
    }

    // MARK: - Entry list

    /// Route selection through the model instead of observing it, so a
    /// programmatic select (create / delete / undo) is not re-applied.
    private var selectionBinding: Binding<String?> {
        Binding(
            get: { model.selectedKey },
            set: { model.select($0) }
        )
    }

    private var entryList: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                TextField("按 key 前缀搜索", text: $model.searchText)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { model.applySearch() }
                Button {
                    model.applySearch()
                } label: {
                    Image(systemName: "magnifyingglass")
                }
                .accessibilityLabel("搜索")
                if !model.searchText.isEmpty {
                    Button {
                        model.clearSearch()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("清除搜索")
                }
            }
            .padding(8)

            List(selection: selectionBinding) {
                ForEach(model.rows) { row in
                    EntryRowView(row: row)
                        .tag(row.key)
                }
            }

            HStack {
                Text("已加载 \(model.rows.count) 条")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if model.hasMore {
                    Button("加载更多") { model.loadMore() }
                        .buttonStyle(.link)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
        }
    }

    // MARK: - Detail

    @ViewBuilder
    private var detail: some View {
        if let form = model.form {
            VSplitView {
                ScrollView {
                    EntryFormView(model: model, form: form)
                        .padding(16)
                }
                .frame(minHeight: 280)

                DiffPane(model: model)
                    .frame(minHeight: 150)
            }
        } else if model.selectedKey != nil {
            DiffPane(model: model)
        } else {
            ContentUnavailableView(
                "选择一个词条",
                systemImage: "text.cursor",
                description: Text("从左侧列表选择词条后，在这里编辑字段或查看差异。")
            )
        }
    }

    // MARK: - Status bar

    private var statusBar: some View {
        HStack(spacing: 14) {
            ChangeCount(label: "新增", count: model.status.added, color: .green)
            ChangeCount(label: "修改", count: model.status.modified, color: .orange)
            ChangeCount(label: "删除", count: model.status.removed, color: .red)
            Text("共 \(model.status.total) 条")
                .font(.callout)
                .foregroundStyle(.secondary)

            Spacer()

            if let notice = model.noticeMessage {
                Text(notice)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Button("丢弃…") {
                isDiscardPresented = true
            }
            .disabled(!model.hasChanges)

            Button("提交…") {
                if model.hasChanges {
                    isCommitPresented = true
                } else {
                    model.alertMessage = "工作区没有改动，不需要提交。"
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

private struct EntryRowView: View {
    let row: DictionaryEditorModel.Row

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.display)
                    .lineLimit(1)
                Text(row.key)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if !row.summary.isEmpty {
                    Text(row.summary)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 4)
            if let change = row.change {
                ChangeBadge(change: change)
            }
        }
        .padding(.vertical, 2)
    }
}

private struct ChangeBadge: View {
    let change: ChangeKind

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.15), in: Capsule())
            .foregroundStyle(color)
    }

    private var title: String {
        switch change {
        case .added: return "新增"
        case .modified: return "修改"
        case .deleted: return "删除"
        }
    }

    private var color: Color {
        switch change {
        case .added: return .green
        case .modified: return .orange
        case .deleted: return .red
        }
    }
}

private struct ChangeCount: View {
    let label: String
    let count: Int
    let color: Color

    var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(color)
                .frame(width: 6, height: 6)
            Text("\(label) \(count)")
                .font(.callout)
        }
        .foregroundStyle(count > 0 ? .primary : .secondary)
    }
}

private struct EntryFormView: View {
    @Bindable var model: DictionaryEditorModel
    let form: EntryEditorForm

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if model.canRenameSelectedEntry {
                LabeledContent("key") {
                    TextField("key", text: binding(\.key, field: "key"))
                        .textFieldStyle(.roundedBorder)
                }
                Text("新词条保存前可以修改 key。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                HStack {
                    Text(form.key)
                        .font(.headline.monospaced())
                    Text("key 不可修改")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            LabeledContent("display") {
                TextField("display", text: binding(\.display, field: "display"))
                    .textFieldStyle(.roundedBorder)
            }

            LabeledContent("phonetic") {
                TextField("多个读音用逗号分隔", text: binding(\.phoneticText, field: "phoneticText"))
                    .textFieldStyle(.roundedBorder)
            }

            Divider()

            HStack {
                Text("释义")
                    .font(.headline)
                Spacer()
                Button {
                    model.addDefinition()
                } label: {
                    Label("添加释义", systemImage: "plus")
                }
                .buttonStyle(.link)
            }

            ForEach(form.definitions) { definition in
                DefinitionEditorView(
                    model: model,
                    id: definition.id,
                    pos: definitionBinding(definition.id, \.pos, field: "pos"),
                    sense: definitionBinding(definition.id, \.sense, field: "sense"),
                    examples: definitionBinding(definition.id, \.examplesText, field: "examples"),
                    labels: definitionBinding(definition.id, \.labelsText, field: "labels")
                )
            }

            if form.definitions.isEmpty {
                Text("还没有释义，点「添加释义」开始。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Divider()

            VStack(alignment: .leading, spacing: 6) {
                Text("raw_html")
                    .font(.headline)
                Text("结构化释义为空时，阅读器会渲染这段 HTML。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: binding(\.rawHTML, field: "rawHTML"))
                    .font(.body.monospaced())
                    .frame(minHeight: 90)
                    .overlay {
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(.quaternary)
                    }
            }

            if let message = model.validationMessage {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
            }

            HStack {
                Spacer()
                Button("保存到工作区") {
                    _ = model.save()
                }
            }
        }
    }

    private func binding(
        _ keyPath: WritableKeyPath<EntryEditorForm, String>,
        field: String
    ) -> Binding<String> {
        Binding(
            get: { model.form?[keyPath: keyPath] ?? "" },
            set: { value in
                model.updateForm(field) { $0[keyPath: keyPath] = value }
            }
        )
    }

    private func definitionBinding(
        _ id: UUID,
        _ keyPath: WritableKeyPath<DefinitionForm, String>,
        field: String
    ) -> Binding<String> {
        Binding(
            get: {
                model.form?.definitions.first { $0.id == id }?[keyPath: keyPath] ?? ""
            },
            set: { value in
                model.updateForm("\(field).\(id.uuidString)") { form in
                    guard let index = form.definitions.firstIndex(where: { $0.id == id }) else {
                        return
                    }
                    form.definitions[index][keyPath: keyPath] = value
                }
            }
        )
    }
}

private struct DefinitionEditorView: View {
    let model: DictionaryEditorModel
    let id: UUID
    @Binding var pos: String
    @Binding var sense: String
    @Binding var examples: String
    @Binding var labels: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("词性（pos）", text: $pos)
                    .textFieldStyle(.roundedBorder)
                Button(role: .destructive) {
                    model.removeDefinition(id: id)
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("删除这条释义")
            }

            TextField("释义（sense）", text: $sense)
                .textFieldStyle(.roundedBorder)

            Text("例句（每行一句）")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextEditor(text: $examples)
                .font(.body)
                .frame(minHeight: 52)
                .overlay {
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(.quaternary)
                }

            TextField("标签（逗号分隔）", text: $labels)
                .textFieldStyle(.roundedBorder)
        }
        .padding(10)
        .background(.quinary, in: RoundedRectangle(cornerRadius: 8))
    }
}

private struct DiffPane: View {
    @Bindable var model: DictionaryEditorModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("字段差异")
                    .font(.headline)
                if let kind = model.diff?.kind {
                    DiffKindBadge(kind: kind)
                }
                Spacer()
                Toggle("仅改动", isOn: $model.showOnlyChanged)
                    .toggleStyle(.switch)
                    .controlSize(.small)
            }
            .padding(.horizontal, 14)
            .padding(.top, 10)

            if model.visibleDiffFields.isEmpty {
                Text(model.diff == nil ? "选择词条后显示差异。" : "没有字段差异。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                Spacer()
            } else {
                ScrollView {
                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                        GridRow {
                            Text("字段").font(.caption.weight(.semibold))
                            Text("修改前").font(.caption.weight(.semibold))
                            Text("修改后").font(.caption.weight(.semibold))
                        }
                        Divider()
                        ForEach(model.visibleDiffFields, id: \.field) { field in
                            GridRow {
                                Text(field.field)
                                    .font(.caption.monospaced())
                                Text(field.before?.displayText ?? "—")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text(field.after?.displayText ?? "—")
                                    .font(.caption)
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.bottom, 10)
                }
            }
        }
    }
}

private struct DiffKindBadge: View {
    let kind: EntryDiffKind

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.15), in: Capsule())
            .foregroundStyle(color)
    }

    private var title: String {
        switch kind {
        case .added: return "新增"
        case .modified: return "修改"
        case .deleted: return "删除"
        case .absent: return "无变化"
        }
    }

    private var color: Color {
        switch kind {
        case .added: return .green
        case .modified: return .orange
        case .deleted: return .red
        case .absent: return .secondary
        }
    }
}

private struct CommitSheet: View {
    @Bindable var model: DictionaryEditorModel
    let onCommitted: () -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var message = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("提交改动")
                .font(.title3.weight(.semibold))
            Text("新增 \(model.status.added) · 修改 \(model.status.modified) · 删除 \(model.status.removed)")
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField("提交信息", text: $message)
                .textFieldStyle(.roundedBorder)
                .onSubmit(commit)
            if let errorText = model.alertMessage {
                Text(errorText)
                    .font(.callout)
                    .foregroundStyle(.red)
            }
            HStack {
                Spacer()
                Button("取消") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("提交") { commit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private func commit() {
        if model.commit(message: message) {
            dismiss()
            onCommitted()
        }
    }
}
#endif
