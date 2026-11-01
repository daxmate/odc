import SwiftUI

/// Renders one dictionary entry, read-only.
public struct EntryDetailView: View {
    private let store: DictionaryStore
    private let explicitKey: String?

    /// Detail column driven by the store's current selection.
    public init(store: DictionaryStore) {
        self.store = store
        self.explicitKey = nil
    }

    /// Detail pushed from a navigation link; loads the key when it appears.
    public init(store: DictionaryStore, key: String) {
        self.store = store
        self.explicitKey = key
    }

    public var body: some View {
        Group {
            if let entry = store.entry {
                entryView(entry)
            } else if store.isLoading {
                ProgressView()
            } else {
                ContentUnavailableView(
                    "选择一个词条",
                    systemImage: "character.book.closed",
                    description: Text("在左侧搜索并选择词条。")
                )
            }
        }
        .task {
            if let explicitKey {
                await store.select(key: explicitKey)
            }
        }
    }

    @ViewBuilder
    private func entryView(_ entry: DictionaryEntry) -> some View {
        if let rawHTML = entry.rawHTML, !rawHTML.isEmpty {
            renderedEntry(entry, html: rawHTML)
        } else {
            ScrollView {
                structuredContent(entry)
                    .padding(24)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .textSelection(.enabled)
        }
    }

    /// The dictionary's own HTML leads; the parsed fields stay one disclosure away.
    private func renderedEntry(_ entry: DictionaryEntry, html: String) -> some View {
        VStack(spacing: 0) {
            HTMLRenderView(
                html: html,
                resources: { key in store.resource(for: key) },
                onEntryLink: { target in
                    Task { await store.select(key: target) }
                }
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            DisclosureGroup("结构化释义") {
                ScrollView {
                    structuredContent(entry)
                        .padding(.top, 8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 260)
                .textSelection(.enabled)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
    }

    private func structuredContent(_ entry: DictionaryEntry) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            header(entry)
            if !entry.defs.isEmpty {
                definitions(entry.defs)
            }
            if !entry.extra.isEmpty {
                extraFields(entry.extra)
            }
        }
    }

    private func header(_ entry: DictionaryEntry) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(entry.display)
                .font(.largeTitle.weight(.semibold))
            if !entry.phonetic.isEmpty {
                Text(entry.phonetic.joined(separator: " · "))
                    .font(.title3.monospaced())
                    .foregroundStyle(.secondary)
            }
            if entry.key != entry.display {
                Text(entry.key)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
    }

    private func definitions(_ defs: [Definition]) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            ForEach(Array(defs.enumerated()), id: \.offset) { index, def in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        Text("\(index + 1).")
                            .font(.callout.monospacedDigit())
                            .foregroundStyle(.secondary)
                        if !def.pos.isEmpty {
                            Text(def.pos)
                                .font(.caption.weight(.medium))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(.quaternary, in: Capsule())
                        }
                        ForEach(def.labels, id: \.self) { label in
                            Text(label)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Text(def.sense)
                        .font(.body)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(def.examples, id: \.self) { example in
                        Text(example)
                            .font(.callout.italic())
                            .foregroundStyle(.secondary)
                            .padding(.leading, 12)
                    }
                }
            }
        }
    }

    private func extraFields(_ extra: [String: JSONValue]) -> some View {
        DisclosureGroup("其他字段") {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(extra.keys.sorted(), id: \.self) { name in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(name)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                        Text(extra[name]?.displayText ?? "")
                            .font(.caption)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 6)
        }
    }
}
