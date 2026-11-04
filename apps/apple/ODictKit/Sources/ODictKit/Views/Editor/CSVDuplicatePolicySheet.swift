#if os(macOS)
import SwiftUI

/// Confirmation shown before an import that would touch existing keys.
struct CSVDuplicatePolicySheet: View {
    @Bindable var model: CSVImportModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("发现重复词条")
                .font(.title3.weight(.semibold))
            Text("导入前需要决定这些 key 怎么处理。")
                .font(.callout)
                .foregroundStyle(.secondary)

            if let summary = model.duplicateSummary {
                VStack(alignment: .leading, spacing: 4) {
                    Text("CSV 内部重复 key：\(summary.internalDuplicates.count) 个（多出 \(summary.internalExtraRows) 行）")
                    Text("库内已存在 key：\(summary.libraryConflicts.count) 个")
                }
                .font(.callout)

                let examples = summary.exampleLines()
                if !examples.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("示例")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        ForEach(examples, id: \.self) { line in
                            Text(line)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }

            Divider()

            Text("处理方式")
                .font(.headline)
            ForEach(CSVDuplicatePolicy.allCases, id: \.self) { policy in
                Button {
                    model.pendingPolicy = policy
                } label: {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: model.pendingPolicy == policy ? "largecircle.fill.circle" : "circle")
                            .foregroundStyle(model.pendingPolicy == policy ? Color.accentColor : Color.secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(policy.title)
                                .foregroundStyle(.primary)
                            Text(policy.explanation)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                    }
                }
                .buttonStyle(.plain)
            }

            Toggle("记住此选择（按库保存）", isOn: $model.rememberPolicy)

            HStack {
                Spacer()
                Button("取消") {
                    model.cancelPolicyPrompt()
                }
                .keyboardShortcut(.cancelAction)
                Button("继续导入") {
                    model.confirmPolicy()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
        .frame(width: 460)
    }
}
#endif
