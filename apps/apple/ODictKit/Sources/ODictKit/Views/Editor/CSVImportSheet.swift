#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct CSVImportSheet: View {
    @Bindable var model: CSVImportModel
    let onFinished: () -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header

            if model.table == nil {
                emptyState
            } else {
                mappingSection
                Divider()
                previewSection
                Divider()
                progressSection
            }

            if let message = model.alertMessage {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
            }

            footer
        }
        .padding(18)
        .frame(minWidth: 820, minHeight: 560)
        .onChange(of: model.mapping) { _, _ in
            model.makePlan()
        }
        .alert(
            "导入失败",
            isPresented: Binding(
                get: { model.alertMessage != nil },
                set: { if !$0 { model.alertMessage = nil } }
            )
        ) {
            Button("好", role: .cancel) { model.alertMessage = nil }
        } message: {
            Text(model.alertMessage ?? "")
        }
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("导入 CSV / Excel")
                    .font(.title3.weight(.semibold))
                Text(model.fileName ?? "选择一份 CSV 或 Excel 文件（.csv / .xlsx），同一 key 的多行会合并为多个释义。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("选择 CSV…") { chooseFile() }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: "tablecells")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("还没有选择文件")
                .font(.headline)
            Button("选择文件…") { chooseFile() }
                .keyboardShortcut(.defaultAction)
        }
        .frame(maxWidth: .infinity, minHeight: 320)
    }

    private var mappingSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("表头映射")
                .font(.headline)
            Text("只有 key 是必填的；其余留空表示不导入该字段。")
                .font(.caption)
                .foregroundStyle(.secondary)

            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 8) {
                GridRow {
                    columnPicker("key（必填）", selection: $model.mapping.key)
                    columnPicker("display", selection: $model.mapping.display)
                }
                GridRow {
                    columnPicker("phonetic", selection: $model.mapping.phonetic)
                    columnPicker("pos", selection: $model.mapping.pos)
                }
                GridRow {
                    columnPicker("sense", selection: $model.mapping.sense)
                    columnPicker("examples", selection: $model.mapping.examples)
                }
                GridRow {
                    columnPicker("labels", selection: $model.mapping.labels)
                    Color.clear.frame(height: 1)
                }
            }
        }
    }

    private func columnPicker(_ title: String, selection: Binding<Int?>) -> some View {
        Picker(title, selection: selection) {
            Text("不使用").tag(Int?.none)
            ForEach(model.header.indices, id: \.self) { index in
                Text(model.header[index].isEmpty ? "第 \(index + 1) 列" : model.header[index])
                    .tag(Int?.some(index))
            }
        }
        .frame(minWidth: 240)
    }

    private var previewSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("预览（前 20 行）")
                    .font(.headline)
                Spacer()
                Text("\(model.totalRows) 行数据")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let note = model.sourceNote {
                Text(note)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            ScrollView([.horizontal, .vertical]) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 12) {
                        ForEach(model.header.indices, id: \.self) { index in
                            Text(model.header[index].isEmpty ? "第 \(index + 1) 列" : model.header[index])
                                .font(.caption.weight(.semibold))
                                .frame(minWidth: 90, alignment: .leading)
                        }
                    }
                    Divider()
                    ForEach(Array(model.previewRows.enumerated()), id: \.offset) { _, row in
                        HStack(spacing: 12) {
                            ForEach(model.header.indices, id: \.self) { index in
                                Text(index < row.count ? row[index] : "")
                                    .font(.caption)
                                    .lineLimit(1)
                                    .frame(minWidth: 90, alignment: .leading)
                            }
                        }
                    }
                }
                .padding(8)
            }
            .frame(height: 170)
            .background(.quinary, in: RoundedRectangle(cornerRadius: 8))
        }
    }

    @ViewBuilder
    private var progressSection: some View {
        switch model.phase {
        case .idle:
            EmptyView()
        case .ready:
            HStack {
                Text("准备导入 \(model.preparedPlan?.entries.count ?? 0) 个词条")
                    .font(.callout)
                Spacer()
            }
        case .importing:
            VStack(alignment: .leading, spacing: 6) {
                ProgressView(
                    value: Double(model.processedRows),
                    total: Double(max(model.totalRows, 1))
                )
                Text("已处理 \(model.processedRows) / \(model.totalRows) 行 · 写入 \(model.importedEntries) 个词条 · 跳过 \(model.skippedRows) 行")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        case .finished:
            VStack(alignment: .leading, spacing: 6) {
                Text("导入完成：写入 \(model.importedEntries) 个词条，跳过 \(model.skippedRows) 行。")
                    .font(.headline)
                ProgressView(
                    value: Double(model.processedRows),
                    total: Double(max(model.totalRows, 1))
                )
                if !model.issues.isEmpty {
                    Text("问题明细（前 5 条）：")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(Array(model.issues.prefix(5).enumerated()), id: \.offset) { _, issue in
                        Text(issue)
                            .font(.caption)
                            .foregroundStyle(.orange)
                    }
                }
            }
        }
    }

    private var footer: some View {
        HStack {
            if model.table != nil {
                Button("重新选择…") { chooseFile() }
            }
            Spacer()
            Button("取消") {
                dismiss()
            }
            .keyboardShortcut(.cancelAction)

            if model.phase == .finished {
                Button("完成") {
                    onFinished()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            } else {
                Button("导入") {
                    model.startImport()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!model.canImport || model.phase == .importing)
            }
        }
    }

    /// `.csv` / `.txt` plus `.xlsx`, looked up by extension so it also works on
    /// macOS 14, which has no static `UTType` constant for xlsx.
    private static var importContentTypes: [UTType] {
        var types: [UTType] = [.commaSeparatedText, .plainText]
        if let xlsx = UTType(filenameExtension: "xlsx") {
            types.append(xlsx)
        }
        return types
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = Self.importContentTypes
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.prompt = "导入"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.load(url: url)
    }
}
#endif
