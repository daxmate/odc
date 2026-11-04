#if os(macOS)
import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct CSVImportSheet: View {
    @Bindable var model: CSVImportModel
    let onFinished: () -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var isTemplateNamePromptPresented = false

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
        .sheet(
            isPresented: Binding(
                get: { model.isPolicyPromptPresented },
                set: { if !$0 { model.cancelPolicyPrompt() } }
            )
        ) {
            CSVDuplicatePolicySheet(model: model)
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

            templateRow

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

    private var templateRow: some View {
        HStack(spacing: 8) {
            Menu {
                if model.templates.isEmpty {
                    Text("还没有模板")
                }
                ForEach(model.templates) { template in
                    Button(template.name) {
                        model.applyTemplate(named: template.name)
                    }
                }
                if let name = model.matchedTemplateName {
                    Divider()
                    Button("删除模板「\(name)」", role: .destructive) {
                        model.deleteTemplate(named: name)
                    }
                }
            } label: {
                Label(
                    model.matchedTemplateName.map { "模板：\($0)" } ?? "映射模板",
                    systemImage: "square.stack.3d.up"
                )
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            Button("保存为模板…") {
                model.templateNameDraft = model.matchedTemplateName ?? ""
                isTemplateNamePromptPresented = true
            }
            .disabled(!model.mapping.isValid)

            if let name = model.matchedTemplateName {
                Text("已套用模板「\(name)」")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .alert("保存映射模板", isPresented: $isTemplateNamePromptPresented) {
            TextField("模板名称", text: $model.templateNameDraft)
            Button("保存") {
                _ = model.saveCurrentMappingAsTemplate(named: model.templateNameDraft)
            }
            Button("取消", role: .cancel) {
                model.templateNameDraft = ""
            }
        } message: {
            Text("同一表头的文件下次导入时会自动套用。")
        }
    }

    private var summaryLine: String {
        var parts = ["新建 \(model.importedEntries) 个词条"]
        if let encoding = model.encodingLabel {
            parts.append("编码 \(encoding)")
        }
        if model.overwrittenEntries > 0 {
            parts.append("覆盖 \(model.overwrittenEntries) 个")
        }
        if model.mergedEntries > 0 {
            parts.append("合并 \(model.mergedEntries) 个")
        }
        parts.append("跳过 \(model.skippedRows) 行")
        return "导入完成：" + parts.joined(separator: "，") + "。"
    }

    private var issueReport: some View {
        let total = model.rowIssues.count
        let shown = min(total, 100)
        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("问题明细 \(total) 条（显示前 \(shown) 条）")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("复制错误报告") {
                    copyErrorReport()
                }
                .font(.caption)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(model.rowIssues.prefix(100))) { issue in
                        Text(issue.displayText)
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if total > 100 {
                        Text("还有 \(total - 100) 条")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .frame(height: 110)
            .background(.quinary, in: RoundedRectangle(cornerRadius: 6))
        }
    }

    private func copyErrorReport() {
        var lines = [
            "ODict 导入错误报告",
            "文件：\(model.fileName ?? "-")",
            "编码：\(model.encodingLabel ?? "-")",
            "问题：\(model.rowIssues.count) 条",
            "",
        ]
        lines.append(contentsOf: model.rowIssues.map(\.displayText))
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(lines.joined(separator: "\n"), forType: .string)
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

            if let encoding = model.encodingLabel {
                Text("Encoding: \(encoding)")
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
                Text("已处理 \(model.processedRows) / \(model.totalRows) 行 · 新建 \(model.importedEntries) · 跳过 \(model.skippedRows)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        case .finished:
            VStack(alignment: .leading, spacing: 6) {
                Text(summaryLine)
                    .font(.headline)
                if !model.rowIssues.isEmpty {
                    issueReport
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
