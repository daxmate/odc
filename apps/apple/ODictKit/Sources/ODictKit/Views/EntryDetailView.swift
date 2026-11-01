import SwiftUI

/// Renders the entry for the current selection, one section per open
/// dictionary, stacked on a single scrolling page.
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
            if store.entrySections.isEmpty {
                if store.isLoading {
                    ProgressView()
                } else if let notice = store.noticeMessage {
                    ContentUnavailableView(
                        "无法显示词条",
                        systemImage: "exclamationmark.triangle",
                        description: Text(notice)
                    )
                } else {
                    ContentUnavailableView(
                        "选择一个词条",
                        systemImage: "character.book.closed",
                        description: Text("在左侧搜索并选择词条。")
                    )
                }
            } else {
                sections
            }
        }
        .task {
            if let explicitKey {
                await store.select(key: explicitKey)
            }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                if let entry = store.entry {
                    bookmarkButton(for: entry)
                }
            }
        }
    }

    /// One section per dictionary that carries the key, separated by a rule.
    private var sections: some View {
        let showsTitles = store.entrySections.count > 1
        return ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let notice = store.noticeMessage {
                    Label(notice, systemImage: "arrow.triangle.branch")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                }
                ForEach(Array(store.entrySections.enumerated()), id: \.element.id) { index, section in
                    if index > 0 {
                        Divider()
                            .padding(.horizontal, 16)
                            .padding(.vertical, 12)
                    }
                    if showsTitles {
                        Text(section.name)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 16)
                            .padding(.top, index == 0 ? 12 : 0)
                            .padding(.bottom, 4)
                    }
                    EntrySectionView(store: store, section: section)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Star toggling for the entry on screen; the label is also its a11y name.
    private func bookmarkButton(for entry: DictionaryEntry) -> some View {
        let isBookmarked = store.isBookmarked(key: entry.key)
        return Button {
            store.toggleBookmark()
        } label: {
            Label(
                isBookmarked ? "取消收藏" : "收藏",
                systemImage: isBookmarked ? "star.fill" : "star"
            )
        }
        .accessibilityLabel(isBookmarked ? "取消收藏" : "收藏")
    }

    /// Message shown in place of the web view, or `nil` when it drew the entry.
    static func fallbackMessage(for outcome: EntryRenderOutcome) -> String? {
        switch outcome {
        case .loaded(let characters) where characters > 0:
            return nil
        case .loaded:
            return "这篇词条没有可显示的正文，已切换到结构化释义。"
        case .failed(let reason):
            return "词条正文加载失败（\(reason)），已切换到结构化释义。"
        }
    }
}


/// One dictionary's share of the detail page.
///
/// The section owns its own key: following a link inside it swaps only this
/// dictionary's entry, so the other dictionaries keep what they showed. Its web
/// view reports its height so the sections share one scroll view.
private struct EntrySectionView: View {
    let store: DictionaryStore
    let section: DictionaryEntrySection

    /// Key this section navigated to on its own; nil while it follows the page.
    @State private var localKey: String?
    @State private var localEntry: DictionaryEntry?
    @State private var localNotice: String?
    /// Set when the web view could not show the entry; the parsed fields take
    /// its place instead of a blank panel.
    @State private var renderFailure: String?
    /// Height the document reported, so the page scrolls as a whole.
    @State private var contentHeight: CGFloat?

    /// Height used until the document reports its own, tall enough to read.
    private static let unmeasuredHeight: CGFloat = 240

    private var entry: DictionaryEntry { localEntry ?? section.entry }

    private var notice: String? { localEntry == nil ? section.notice : localNotice }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let notice {
                Label(notice, systemImage: "arrow.triangle.branch")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
            }
            if let renderFailure {
                VStack(alignment: .leading, spacing: 16) {
                    Label(renderFailure, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    structuredContent(entry)
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            } else if let rawHTML = entry.rawHTML, !rawHTML.isEmpty {
                renderedEntry(entry, html: rawHTML)
            } else {
                structuredContent(entry)
                    .padding(24)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
        }
        .onChange(of: section.entry) { resetLocalNavigation() }
        .onChange(of: store.selectedKey) { resetLocalNavigation() }
        .task(id: localKey) {
            await followLocalLink()
        }
    }

    /// The dictionary's own HTML leads; the parsed fields stay one disclosure away.
    private func renderedEntry(_ entry: DictionaryEntry, html: String) -> some View {
        VStack(spacing: 0) {
            HTMLRenderView(
                html: html,
                resources: { key in store.resource(for: key, in: section.dictionaryID) },
                onEntryLink: { target in localKey = target },
                onOutcome: { outcome in
                    renderFailure = EntryDetailView.fallbackMessage(for: outcome)
                },
                onContentHeight: { height in update(height: height) }
            )
            .frame(maxWidth: .infinity)
            .frame(height: contentHeight ?? Self.unmeasuredHeight)
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
        .onChange(of: html) { renderFailure = nil }
    }

    /// Drop this section's own navigation so it follows the page again.
    private func resetLocalNavigation() {
        localKey = nil
        localEntry = nil
        localNotice = nil
        renderFailure = nil
        contentHeight = nil
    }

    /// Follow a link that was tapped inside this section only.
    private func followLocalLink() async {
        guard let localKey else { return }
        guard let resolved = store.entrySection(for: localKey, in: section.dictionaryID) else {
            localNotice = "这本词典里没有「\(localKey)」"
            return
        }
        localEntry = resolved.entry
        localNotice = resolved.notice
        renderFailure = nil
        contentHeight = nil
    }

    /// Keep the reported height steady; tiny differences would loop the layout.
    private func update(height: CGFloat) {
        guard height.isFinite, height > 0 else { return }
        if let contentHeight, abs(contentHeight - height) < 0.5 { return }
        contentHeight = height
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
