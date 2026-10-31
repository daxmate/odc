import Foundation
import Observation

/// Observable state for the read-only dictionary UI.
@MainActor
@Observable
public final class DictionaryStore {
    /// Where the imported copy lives (inside the app container).
    public private(set) var documentURL: URL?
    /// Name of the file the reader picked, used for display.
    public private(set) var documentName: String?
    /// Current search text (not trimmed).
    public private(set) var query: String = ""
    /// Prefix-search results for the current query.
    public private(set) var hits: [SearchHit] = []
    /// Key of the entry currently shown.
    public private(set) var selectedKey: String?
    /// The entry for the current selection, once loaded.
    public private(set) var entry: DictionaryEntry?
    /// User-facing error text, cleared by `dismissError()`.
    public private(set) var errorMessage: String?
    /// True while a document is being imported or a search is running.
    public private(set) var isLoading = false

    @ObservationIgnored private var dictionary: OdcDictionary?
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var importTask: Task<Void, Never>?

    /// Search debounce, in milliseconds.
    private static let debounceMilliseconds = 200

    public init() {}

    // MARK: - Opening and closing

    /// Import `url` into the sandbox and open the copy.
    public func open(url: URL) {
        importTask?.cancel()
        importTask = Task { [weak self] in
            await self?.performOpen(url: url)
        }
    }

    /// Close the current dictionary and delete its imported copy.
    public func close() {
        importTask?.cancel()
        searchTask?.cancel()
        closeDictionary()
        if let documentURL {
            DocumentLibrary.removeCopy(at: documentURL)
        }
        documentURL = nil
        documentName = nil
        query = ""
        hits = []
        selectedKey = nil
        entry = nil
        errorMessage = nil
        isLoading = false
    }

    private func performOpen(url: URL) async {
        errorMessage = nil
        isLoading = true
        defer { isLoading = false }

        let previous = documentURL
        do {
            let copy = try await Task.detached(priority: .userInitiated) {
                try DocumentLibrary.copyDocument(at: url)
            }.value
            if Task.isCancelled {
                DocumentLibrary.removeCopy(at: copy)
                return
            }
            let opened: OdcDictionary
            do {
                opened = try OdcDictionary(url: copy)
            } catch {
                DocumentLibrary.removeCopy(at: copy)
                throw error
            }
            closeDictionary()
            dictionary = opened
            documentURL = copy
            documentName = url.lastPathComponent
            query = ""
            hits = []
            selectedKey = nil
            entry = nil
            if let previous {
                DocumentLibrary.removeCopy(at: previous)
            }
        } catch is CancellationError {
            return
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Searching

    /// Update the search text and schedule a debounced prefix search.
    public func setQuery(_ text: String) {
        query = text
        searchTask?.cancel()

        guard dictionary != nil else {
            hits = []
            return
        }

        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            hits = []
            selectedKey = nil
            entry = nil
            return
        }

        searchTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(Self.debounceMilliseconds))
            } catch {
                return
            }
            guard let self, !Task.isCancelled else { return }
            await self.runSearch(trimmed)
        }
    }

    private func runSearch(_ text: String) async {
        guard let dictionary else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let results = try dictionary.search(text)
            hits = results
            if let selectedKey, !results.contains(where: { $0.key == selectedKey }) {
                self.selectedKey = nil
                entry = nil
            }
        } catch {
            hits = []
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Selection

    /// Load the entry for `key`, replacing any current selection.
    public func select(key: String) async {
        guard let dictionary else { return }
        selectedKey = key
        entry = nil
        errorMessage = nil
        do {
            let loaded = try dictionary.entry(for: key)
            guard selectedKey == key else { return }
            entry = loaded
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Errors

    /// Surface an error raised outside the store (for example by a picker).
    public func present(error: any Error) {
        errorMessage = error.localizedDescription
    }

    public func dismissError() {
        errorMessage = nil
    }

    private func closeDictionary() {
        dictionary?.close()
        dictionary = nil
    }
}
