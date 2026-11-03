import SwiftUI
import UniformTypeIdentifiers
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Publisher keypair settings: generate, import, inspect and remove.
///
/// Shown as the macOS Settings scene and reachable from the editor toolbar;
/// the same view compiles for iOS so the reader can inspect an identity later.
public struct PublisherSettingsView: View {
    @State private var model: PublisherSettingsModel
    @State private var isImporterPresented = false
    @State private var isDeleting = false
    @State private var isReplacing = false
    @State private var importError: String?

    public init(model: PublisherSettingsModel = PublisherSettingsModel()) {
        _model = State(initialValue: model)
    }

    public var body: some View {
        Form {
            Section("发布者身份") {
                if let identity = model.identity {
                    LabeledContent("名称", value: identity.name)
                    LabeledContent("指纹") {
                        Text(identity.fingerprint)
                            .font(.system(.body, design: .monospaced))
                            .textSelection(.enabled)
                    }
                    LabeledContent("公钥") {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(identity.publicKeyHex)
                                .font(.system(.caption, design: .monospaced))
                                .lineLimit(2)
                                .truncationMode(.middle)
                                .textSelection(.enabled)
                            Button("复制公钥") { copy(identity.publicKeyHex) }
                                .buttonStyle(.borderless)
                        }
                    }
                } else {
                    Text("尚未配置发布者身份。生成或导入一个私钥后才能发布。")
                        .foregroundStyle(.secondary)
                }

                TextField("名称", text: $model.name, prompt: Text("用于发布记录"))
                    .disabled(model.isBusy)
            }

            Section {
                if model.hasIdentity {
                    Button("重新生成密钥…") { isReplacing = true }
                        .disabled(model.isBusy)
                } else {
                    Button("生成新密钥") { model.generate() }
                        .disabled(model.isBusy)
                }
                Button("导入私钥…") { isImporterPresented = true }
                    .disabled(model.isBusy)
                if model.hasIdentity {
                    Button("移除密钥…", role: .destructive) { isDeleting = true }
                        .disabled(model.isBusy)
                }
            } footer: {
                Text("私钥只保存在系统钥匙串中；已发布的工件不会随密钥删除而消失。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let message = model.errorMessage ?? importError {
                Section {
                    Text(message)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .formStyle(.grouped)
        .fileImporter(
            isPresented: $isImporterPresented,
            allowedContentTypes: allowedContentTypes,
            allowsMultipleSelection: false
        ) { result in
            handleImport(result)
        }
        .confirmationDialog("移除发布者密钥？", isPresented: $isDeleting, titleVisibility: .visible) {
            Button("移除密钥", role: .destructive) { model.delete() }
            Button("取消", role: .cancel) {}
        } message: {
            Text("移除后无法再用这个密钥签名；已发布的发布包和信任记录不受影响。")
        }
        .confirmationDialog("替换现有密钥？", isPresented: $isReplacing, titleVisibility: .visible) {
            Button("生成并替换", role: .destructive) { model.generate() }
            Button("取消", role: .cancel) {}
        } message: {
            Text("当前私钥会被覆盖，旧私钥签名的工件将无法再与这个身份对应。")
        }
    }

    private var allowedContentTypes: [UTType] {
        var types: [UTType] = [.plainText]
        for suffix in ["pem", "key"] {
            if let type = UTType(filenameExtension: suffix) {
                types.append(type)
            }
        }
        return types
    }

    private func handleImport(_ result: Result<[URL], Error>) {
        do {
            guard let url = try result.get().first else {
                importError = "没有选择文件。"
                return
            }
            let didAccess = url.startAccessingSecurityScopedResource()
            defer {
                if didAccess { url.stopAccessingSecurityScopedResource() }
            }
            let data = try Data(contentsOf: url)
            guard let text = String(data: data, encoding: .utf8) else {
                importError = "这个文件不是 UTF-8 文本。"
                return
            }
            importError = nil
            model.importKey(pem: text)
        } catch {
            importError = error.localizedDescription
        }
    }

    private func copy(_ text: String) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #else
        UIPasteboard.general.string = text
        #endif
    }
}
