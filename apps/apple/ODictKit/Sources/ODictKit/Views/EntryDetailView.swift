import SwiftUI

/// Renders the entry for the current selection: one page for every open
/// dictionary that carries the key, scrolled by the web view itself.
public struct EntryDetailView: View {
    private let store: DictionaryStore
    private let explicitKey: String?

    /// Entry a section shows after a link inside it was followed. Only that
    /// section changes; the rest of the page keeps the selection.
    @State private var overrides: [UUID: DictionaryEntry] = [:]
    /// Note from the last follow-up link, keyed by dictionary.
    @State private var notices: [UUID: String?] = [:]
    /// Set when the web view could not show the page; the parsed fields take
    /// its place instead of a blank panel.
    @State private var renderFailure: String?
    /// Memoises the built document so a redraw does not re-read stylesheets.
    @State private var cache = PageCache()

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
                }
            } else {
                page
            }
        }
        .task {
            if let explicitKey {
                await store.select(key: explicitKey)
            }
        }
        .onChange(of: pageIdentity) { resetFollowUps() }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                if let entry = store.entry {
                    bookmarkButton(for: entry)
                }
            }
        }
    }

    // MARK: - The page

    private var sections: [PageSectionState] {
        store.entrySections.enumerated().map { index, section in
            PageSectionState(
                index: index,
                section: section,
                entry: overrides[section.dictionaryID] ?? section.entry,
                notice: notices[section.dictionaryID] ?? section.notice
            )
        }
    }

    /// Sections the web view can draw: the ones that carry entry markup.
    private var htmlSections: [PageSectionState] {
        sections.filter { EntryHTML.hasMarkup($0.entry.rawHTML) }
    }

    /// Sections with no markup of their own; they keep the parsed layout.
    private var structuredSections: [PageSectionState] {
        sections.filter { !EntryHTML.hasMarkup($0.entry.rawHTML) }
    }

    private var showsDictionaryNames: Bool { sections.count > 1 }

    private var page: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let notice = store.noticeMessage {
                Label(notice, systemImage: "arrow.triangle.branch")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
            }
            if renderFailure == nil, let document = pageDocument {
                ForEach(htmlSections.filter { $0.notice != nil }) { state in
                    if let notice = state.notice {
                        Label(notice, systemImage: "arrow.triangle.branch")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 4)
                    }
                }
                HTMLRenderView(
                    document: document,
                    resources: { index, key in resource(for: key, section: index) },
                    onEntryLink: { index, target in followLink(section: index, target: target) },
                    onOutcome: { outcome in
                        renderFailure = EntryDetailView.fallbackMessage(for: outcome)
                    }
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                if !structuredSections.isEmpty {
                    structuredScroll
                        .frame(maxHeight: 280)
                }
                disclosureList
            } else {
                fallbackScroll
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// Sections without markup, under the page the web view draws.
    private var structuredScroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                ForEach(structuredSections) { state in
                    VStack(alignment: .leading, spacing: 8) {
                        if showsDictionaryNames {
                            dictionaryTitle(state)
                        }
                        if let notice = state.notice {
                            Text(notice)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        StructuredEntryContent(entry: state.entry)
                    }
                    .padding(.horizontal, 24)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                }
            }
            .padding(.vertical, 16)
        }
        .frame(maxWidth: .infinity)
    }

    /// Everything as parsed fields, used when the web view cannot show the page.
    private var fallbackScroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let renderFailure {
                    Label(renderFailure, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 24)
                }
                ForEach(sections) { state in
                    VStack(alignment: .leading, spacing: 8) {
                        if showsDictionaryNames {
                            dictionaryTitle(state)
                        }
                        if let notice = state.notice {
                            Text(notice)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        StructuredEntryContent(entry: state.entry)
                    }
                    .padding(.horizontal, 24)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                }
            }
            .padding(.vertical, 16)
        }
    }

    /// The parsed fields behind each section that the web view draws.
    private var disclosureList: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(htmlSections) { state in
                DisclosureGroup {
                    ScrollView {
                        StructuredEntryContent(entry: state.entry)
                            .padding(.top, 8)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 260)
                    .textSelection(.enabled)
                } label: {
                    Text(showsDictionaryNames ? "结构化释义 · \(state.section.name)" : "结构化释义")
                        .font(.callout)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func dictionaryTitle(_ state: PageSectionState) -> some View {
        Text(state.section.name)
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
    }

    // MARK: - The document

    /// Identity of what the store asked for. A new key or a different set of
    /// dictionaries drops the follow-ups, which belong to the old page.
    private var pageIdentity: String {
        store.entrySections
            .map { "\($0.dictionaryID.uuidString)#\($0.entry.key)" }
            .joined(separator: "|")
    }

    private func resetFollowUps() {
        overrides = [:]
        notices = [:]
        renderFailure = nil
        cache.signature = nil
        cache.document = nil
    }

    /// Builds the one page the web view loads, or nil when no section carries
    /// markup. Rebuilt only when an entry changes: the memo keeps the
    /// stylesheet reads off the redraw path.
    private var pageDocument: String? {
        let inputs = htmlSections
        guard !inputs.isEmpty else { return nil }
        let signature = inputs
            .map { state in
                "\(state.index)|\(state.section.dictionaryID.uuidString)|"
                    + "\(state.entry.key)|\(state.entry.rawHTML?.hashValue ?? 0)"
            }
            .joined(separator: "\n")
        if cache.signature == signature {
            return cache.document
        }
        let built = inputs.map { state in
            let inlined = EntryHTML.inlineStylesheets(
                in: state.entry.rawHTML ?? "",
                resource: { key in
                    store.resource(for: key, in: state.section.dictionaryID)
                }
            )
            return EntryHTML.PageSection(
                dictionaryID: state.section.dictionaryID.uuidString,
                name: state.section.name,
                html: inlined.html,
                stylesheets: inlined.stylesheets
            )
        }
        let document = built.isEmpty ? nil : EntryHTML.page(built)
        cache.signature = signature
        cache.document = document
        return document
    }

    /// Serves one key from the dictionary behind section index.
    private func resource(for key: String, section index: Int) -> Data? {
        guard let state = sections.first(where: { $0.index == index })
            ?? sections.first
        else { return nil }
        return store.resource(for: key, in: state.section.dictionaryID)
    }

    /// Follows a link tapped inside one section, leaving the others alone.
    private func followLink(section index: Int, target: String) {
        guard let state = sections.first(where: { $0.index == index }) ?? sections.first else {
            return
        }
        guard let resolved = store.entrySection(for: target, in: state.section.dictionaryID) else {
            notices[state.section.dictionaryID] = "这本词典里没有「\(target)」"
            cache.signature = nil
            return
        }
        overrides[state.section.dictionaryID] = resolved.entry
        notices[state.section.dictionaryID] = resolved.notice
        renderFailure = nil
        cache.signature = nil
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

/// Where a section stands: the store's entry, or the one a link led to.
private struct PageSectionState: Identifiable {
    let index: Int
    let section: DictionaryEntrySection
    let entry: DictionaryEntry
    let notice: String?

    var id: Int { index }
}

/// Memoises the built page between redraws.
@MainActor
private final class PageCache {
    var signature: String?
    var document: String?
}

/// One dictionary's parsed fields: the fallback for an entry with no markup,
/// and the content behind each section's disclosure.
private struct StructuredEntryContent: View {
    let entry: DictionaryEntry

    var body: some View {
        structuredContent(entry)
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
