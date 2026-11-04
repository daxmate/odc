#if os(macOS)
import ODictCore
import SwiftUI

/// The editor's "changes" panel: every working-set change relative to HEAD,
/// the field-level diff of the selected entry, and the commit form.
///
/// The editor re-reads HEAD before presenting this sheet and again whenever
/// the app becomes active, so the list never hides a foreign commit.
struct ChangesSheet: View {
    @Bindable var model: WorkingChangesModel
    @Bindable var editor: DictionaryEditorModel

    @Environment(\.dismiss) private var dismiss
    @State private var isDiscardConfirmPresented = false

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            HSplitView {
                changeList
                    .frame(minWidth: 260, idealWidth: 300, maxWidth: 420)
                detail
                    .frame(minWidth: 380)
            }
            Divider()
            footer
        }
        .frame(minWidth: 760, idealWidth: 860, minHeight: 480, idealHeight: 560)
        .task { model.refresh() }
        .confirmationDialog(
            "放弃全部未提交的更改？",
            isPresented: $isDiscardConfirmPresented,
            titleVisibility: .visible
        ) {
            Button("放弃全部更改", role: .destructive) {
                if model.discardAll() {
                    editor.reload()
                }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("所有未提交的新增、修改和删除都会丢失，无法恢复。")
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 10) {
            Text("变更")
                .font(.title3.weight(.semibold))
            Text(summary)
                .font(.callout)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                model.refresh()
            } label: {
                Label("刷新", systemImage: "arrow.clockwise")
            }
            .disabled(model.isRefreshing)
            .help("重新读取工作区")
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
    }

    private var summary: String {
        guard model.hasChanges else { return "没有未提交的改动" }
        return "新增 \(model.status.added) · 修改 \(model.status.modified) · 删除 \(model.status.removed)"
    }

    // MARK: - List

    @ViewBuilder
    private var changeList: some View {
        if model.sections.isEmpty {
            ContentUnavailableView(
                "没有未提交的改动",
                systemImage: "checkmark.circle",
                description: Text("工作区与 HEAD 一致。")
            )
        } else {
            List(selection: selection) {
                ForEach(model.sections) { section in
                    Section {
                        ForEach(section.items) { item in
                            ChangeRowLabel(item: item)
                                .tag(item.key)
                        }
                    } header: {
                        ChangeSectionHeader(section: section)
                    }
                }
            }
            .listStyle(.sidebar)
        }
    }

    private var selection: Binding<String?> {
        Binding(
            get: { model.selectedKey },
            set: { model.select($0) }
        )
    }

    // MARK: - Detail

    @ViewBuilder
    private var detail: some View {
        if model.selectedKey == nil {
            ContentUnavailableView(
                "选择一个词条",
                systemImage: "text.magnifyingglass",
                description: Text("选中左侧的变更可查看字段级差异。")
            )
        } else {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 8) {
                    Text(model.selectedKey ?? "")
                        .font(.headline)
                    if let kind = selectedKind {
                        ChangeKindBadge(kind: kind)
                    }
                    Spacer()
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)

                Divider()

                if model.fieldRows.isEmpty {
                    Text("没有可显示的字段差异。")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(16)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                } else {
                    ScrollView {
                        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                            GridRow {
                                Text("字段").font(.caption.weight(.semibold))
                                Text("修改前").font(.caption.weight(.semibold))
                                Text("修改后").font(.caption.weight(.semibold))
                            }
                            Divider()
                            ForEach(model.fieldRows) { row in
                                GridRow {
                                    Text(row.field)
                                        .font(.caption.monospaced())
                                        .foregroundStyle(.secondary)
                                    Text(row.before)
                                        .font(.caption)
                                        .foregroundStyle(row.changed ? .primary : .secondary)
                                    Text(row.after)
                                        .font(.caption)
                                        .foregroundStyle(row.changed ? .primary : .secondary)
                                }
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                    }
                }
            }
        }
    }

    private var selectedKind: ChangeKind? {
        guard let key = model.selectedKey else { return nil }
        for section in model.sections {
            if section.items.contains(where: { $0.key == key }) {
                return section.kind
            }
        }
        return nil
    }

    // MARK: - Footer

    private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("摘要")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                TextField("提交信息", text: $model.commitMessage)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(commit)
            }

            if let error = model.errorMessage {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.red)
            }

            HStack(spacing: 10) {
                Button("放弃全部…") {
                    isDiscardConfirmPresented = true
                }
                .disabled(!model.hasChanges || model.isCommitting)
                Spacer()
                Button("关闭") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("提交") { commit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canCommit)
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
    }

    private func commit() {
        guard model.commit() else { return }
        if let hash = model.lastCommitHash {
            editor.didCommit(hash: hash)
        }
        dismiss()
    }

}

// MARK: - Rows

private struct ChangeRowLabel: View {
    let item: WorkingChangeItem

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: symbol)
                .foregroundStyle(color)
            Text(item.key)
                .font(.callout)
            Spacer()
        }
        .contentShape(Rectangle())
    }

    private var symbol: String {
        switch item.kind {
        case .added: "plus.circle.fill"
        case .modified: "pencil.circle.fill"
        case .deleted: "minus.circle.fill"
        }
    }

    private var color: Color {
        switch item.kind {
        case .added: .green
        case .modified: .orange
        case .deleted: .red
        }
    }
}

private struct ChangeSectionHeader: View {
    let section: WorkingChangeSection

    var body: some View {
        HStack {
            Text(title)
            Spacer()
            Text("\(section.count)")
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
    }

    private var title: String {
        switch section.kind {
        case .added: "新增"
        case .modified: "修改"
        case .deleted: "删除"
        }
    }
}

private struct ChangeKindBadge: View {
    let kind: ChangeKind

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
        case .added: "新增"
        case .modified: "修改"
        case .deleted: "删除"
        }
    }

    private var color: Color {
        switch kind {
        case .added: .green
        case .modified: .orange
        case .deleted: .red
        }
    }
}
#endif
