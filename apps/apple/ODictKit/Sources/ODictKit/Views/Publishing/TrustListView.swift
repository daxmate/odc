import SwiftUI

/// Trusted publishers of one repository: list, add by key and remove.
///
/// The repository is opened per call, so the sheet never holds a handle that
/// could outlive the editor window.
public struct TrustListView: View {
    private let repoPath: String

    @State private var publishers: [Publisher] = []
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var newKey = ""
    @State private var newName = ""
    @State private var pendingRemoval: Publisher?

    public init(repoPath: String) {
        self.repoPath = repoPath
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("受信任的发布者")
                .font(.title3.weight(.semibold))

            if publishers.isEmpty, !isLoading {
                Text("还没有受信任的发布者。首次导入已签名工件时会请求信任。")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            List {
                ForEach(publishers) { publisher in
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(publisher.displayName)
                            Text(publisher.fingerprint)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                            Text("来源：\(publisher.source.displayName)  添加于 \(addedText(publisher.addedAt))")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("移除", role: .destructive) {
                            pendingRemoval = publisher
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            .frame(minHeight: 180)

            Divider()

            VStack(alignment: .leading, spacing: 6) {
                Text("手动添加")
                    .font(.headline)
                TextField("公钥（64 位十六进制）", text: $newKey)
                    .font(.system(.body, design: .monospaced))
                    .textFieldStyle(.roundedBorder)
                TextField("名称（可选）", text: $newName)
                    .textFieldStyle(.roundedBorder)
                HStack {
                    Button("添加信任") { add() }
                        .disabled(newKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    Button("刷新") { reload() }
                    if isLoading { ProgressView().controlSize(.small) }
                }
            }

            if let message = errorMessage {
                Text(message)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
        .frame(minWidth: 520, minHeight: 460)
        .task { reload() }
        .confirmationDialog(
            "移除这个发布者？",
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { if !$0 { pendingRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingRemoval
        ) { publisher in
            Button("移除", role: .destructive) { remove(publisher) }
            Button("取消", role: .cancel) { pendingRemoval = nil }
        } message: { publisher in
            Text("移除后 \(publisher.displayName) 签名的新工件会被拒绝；已发布的历史不受影响。")
        }
    }

    private func reload() {
        isLoading = true
        errorMessage = nil
        do {
            publishers = try ODCPublishing.publishers(repoPath: repoPath)
        } catch {
            errorMessage = error.localizedDescription
        }
        isLoading = false
    }

    private func add() {
        let key = newKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else {
            errorMessage = "请填写公钥（64 位十六进制）。"
            return
        }
        do {
            let publisher = try ODCPublishing.trustAdd(
                repoPath: repoPath,
                publicKeyHex: key,
                name: newName.isEmpty ? nil : newName
            )
            errorMessage = "已信任 \(publisher.displayName)。"
            newKey = ""
            newName = ""
            reload()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func remove(_ publisher: Publisher) {
        pendingRemoval = nil
        do {
            _ = try ODCPublishing.trustRemove(repoPath: repoPath, publicKeyHex: publisher.publicKeyHex)
            reload()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func addedText(_ date: Date?) -> String {
        guard let date else { return "—" }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}
