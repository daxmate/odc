#if os(macOS)
import AppKit
import SwiftUI

/// Publish wizard for the desktop editor.
///
/// The sheet stays open on failure so the user can fix the label or the
/// output directory and retry; it switches to a report screen on success.
public struct PublishSheet: View {
    @Bindable private var model: PublishModel
    private let onOpenSettings: () -> Void

    @Environment(\.dismiss) private var dismiss

    public init(model: PublishModel, onOpenSettings: @escaping () -> Void = {}) {
        self.model = model
        self.onOpenSettings = onOpenSettings
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            if let report = model.report {
                successView(report)
            } else {
                form
            }
            Divider()
            footer
        }
        .frame(minWidth: 540, minHeight: 520)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("发布词典")
                .font(.title2.weight(.semibold))
            Text(model.libraryName)
                .foregroundStyle(.secondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var form: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                identityRow

                VStack(alignment: .leading, spacing: 4) {
                    Text("版本标签").font(.headline)
                    TextField("例如 2026.11.03", text: $model.label)
                        .textFieldStyle(.roundedBorder)
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text("发布说明").font(.headline)
                    TextEditor(text: $model.notes)
                        .font(.body)
                        .frame(height: 84)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(.quaternary, lineWidth: 1)
                        )
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text("输出目录").font(.headline)
                    HStack {
                        Text(model.outDir?.path ?? "尚未选择")
                            .font(.system(.callout, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(model.outDir == nil ? .secondary : .primary)
                        Spacer()
                        Button("选择…") { chooseOutputDirectory() }
                    }
                }

                Toggle("同时生成增量包", isOn: $model.includePatch)
                    .disabled(!model.canIncludePatch)
                Text(patchCaption)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                if let message = model.errorMessage {
                    Text(message)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var identityRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("签名身份").font(.headline)
            if let identity = model.identity {
                Text("\(identity.name) · \(identity.fingerprint)")
                    .font(.system(.callout, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
            } else {
                Text("尚未配置发布者密钥，先生成或导入一个私钥。")
                    .foregroundStyle(.red)
                Button("打开发布者设置…") {
                    onOpenSettings()
                    dismiss()
                }
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            if model.isPublishing {
                ProgressView().controlSize(.small)
                Text("正在发布…").foregroundStyle(.secondary)
            }
            Spacer()
            if model.report != nil {
                Button("关闭") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("取消") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("发布") {
                    Task { await model.publish() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!model.hasIdentity || model.isPublishing)
            }
        }
        .padding(16)
    }

    private func successView(_ report: PublishReport) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Label("发布完成", systemImage: "checkmark.seal.fill")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(.green)

                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                    GridRow {
                        Text("发布者").foregroundStyle(.secondary)
                        Text(report.publisherFingerprint)
                            .font(.system(.callout, design: .monospaced))
                    }
                    GridRow {
                        Text("HEAD").foregroundStyle(.secondary)
                        Text(String(report.head.prefix(12)))
                            .font(.system(.callout, design: .monospaced))
                    }
                    if let base = report.base {
                        GridRow {
                            Text("增量基线").foregroundStyle(.secondary)
                            Text(String(base.prefix(12)))
                                .font(.system(.callout, design: .monospaced))
                        }
                    }
                    GridRow {
                        Text("输出目录").foregroundStyle(.secondary)
                        Text(report.outDir)
                            .font(.system(.callout, design: .monospaced))
                            .lineLimit(2)
                    }
                }

                Text("工件").font(.headline)
                ForEach(report.artifacts) { artifact in
                    HStack {
                        Text(artifact.fileName)
                            .font(.system(.callout, design: .monospaced))
                        Spacer()
                        Text(artifact.kind).foregroundStyle(.secondary)
                        Text(artifact.size.formatted(.byteCount(style: .file)))
                            .foregroundStyle(.secondary)
                    }
                    .font(.callout)
                }

                Button("在 Finder 中显示") {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: report.outDir)])
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var patchCaption: String {
        guard let last = model.lastRecord else {
            return "这个库还没有上次发布记录，首次发布只能生成完整包。"
        }
        return "基线为上次发布：\(last.shortHead)（\(last.label)）"
    }

    private func chooseOutputDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "选择"
        panel.message = "选择发布工件的输出目录"
        if panel.runModal() == .OK, let url = panel.url {
            model.outDir = url
        }
    }
}
#endif
