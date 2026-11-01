import Foundation
import Observation
import OSLog

/// Library diagnostics; stale entries are dropped quietly instead of via alerts.
private let libraryLog = Logger(subsystem: "com.daxmate.odict", category: "library")

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
    /// Stage of a running MDX conversion, `nil` when idle.
    public private(set) var importPhase: MDXImportPhase?
    /// Summary shown once an MDX conversion finished, until dismissed.
    public private(set) var importSummary: String?
    /// Dictionaries copied into the app's storage.
    public private(set) var library = LibraryState()
    /// True while duplicate copies are being moved out of the library.
    public private(set) var isDeduplicating = false

    @ObservationIgnored private var dictionary: OdcDictionary?
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var importTask: Task<Void, Never>?
    @ObservationIgnored private var conversionTask: Task<Void, Never>?
    @ObservationIgnored private var restoreTask: Task<Void, Never>?
    /// Identifier of the library entry backing `documentURL`.
    @ObservationIgnored private var openItemID: UUID?
    @ObservationIgnored private var hasRestored = false
    /// Library root; `nil` means the app's Application Support folder.
    private let container: URL?

    /// Search debounce, in milliseconds.
    private static let debounceMilliseconds = 200

    /// - Parameter container: Override for tests; defaults to Application Support.
    public init(container: URL? = nil) {
        self.container = container
    }

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
        conversionTask?.cancel()
        searchTask?.cancel()
        closeDictionary()
        if let documentURL {
            DocumentLibrary.removeCopy(at: documentURL)
        }
        if let openItemID, let directory = try? libraryDirectoryURL() {
            LibraryStore.remove(id: openItemID, from: &library)
            saveLibrary(in: directory)
        }
        openItemID = nil
        documentURL = nil
        documentName = nil
        query = ""
        hits = []
        selectedKey = nil
        entry = nil
        errorMessage = nil
        isLoading = false
        importPhase = nil
        importSummary = nil
    }

    private func performOpen(url: URL, source: LibrarySourceDescriptor? = nil) async {
        errorMessage = nil
        isLoading = true
        defer { isLoading = false }

        do {
            let directory = try libraryDirectoryURL()
            let descriptor = try await sourceDescriptor(for: url, override: source)
            if Task.isCancelled {
                return
            }

            // The same file already imported: reopen the copy instead of
            // paying for a second 200 MB duplicate.
            if let existing = LibraryStore.item(matching: descriptor, in: library) {
                if FileManager.default.fileExists(
                    atPath: LibraryStore.openTarget(for: existing, in: directory).path
                ) {
                    try await openExisting(existing, in: directory)
                    importSummary = "这本词典已在库中，已直接打开。"
                    return
                }
                forgetStaleEntry(existing, in: directory)
            }

            // A copy made by a build that had no library yet: adopt it rather
            // than writing the same dictionary a second time.
            if let orphan = try LibraryStore.orphanCopy(
                in: directory,
                matching: descriptor,
                state: library
            ) {
                try register(orphan, in: directory)
                try await openExisting(orphan, in: directory)
                importSummary = "这本词典已在库中，已直接打开。"
                return
            }

            let copy = try await Task.detached(priority: .userInitiated) {
                try DocumentLibrary.copyDocument(at: url, into: directory)
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
            if Task.isCancelled {
                opened.close()
                DocumentLibrary.removeCopy(at: copy)
                return
            }
            let item = LibraryItem(
                id: Self.identifier(ofCopy: copy),
                displayName: descriptor.name,
                storedName: copy.lastPathComponent,
                sourceName: descriptor.name,
                sourceBytes: descriptor.bytes,
                sourceHash: descriptor.contentHash
            )
            closeDictionary()
            dictionary = opened
            documentURL = copy
            documentName = item.displayName
            openItemID = item.id
            query = ""
            hits = []
            selectedKey = nil
            entry = nil
            do {
                // Register only once the copy opens, and drop the copy again if
                // the registration cannot be saved: no half-recorded imports.
                try register(item, in: directory)
            } catch {
                closeDictionary()
                documentURL = nil
                documentName = nil
                openItemID = nil
                DocumentLibrary.removeCopy(at: copy)
                throw error
            }
        } catch is CancellationError {
            return
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Importing MDX

    /// Open `url`, converting an MDX dictionary into `.odc` first.
    public func openDocument(url: URL) {
        if url.pathExtension.lowercased() == "mdx" {
            importMDX(url: url)
        } else if isDirectory(url), !FileManager.default.fileExists(
            atPath: url.appending(path: "odc.db").path
        ), let mdx = MDXImporter.singleMDX(in: url) {
            // A picked folder that is not a repository but holds one MDX:
            // import that dictionary, so its sibling MDD volumes are readable.
            importMDX(url: mdx)
        } else {
            open(url: url)
        }
    }

    /// Whether `url` points at a directory on disk.
    private func isDirectory(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
    }

    /// Convert `url` into a standalone `.odc` document and open the result.
    ///
    /// The conversion runs off the main actor. The store publishes the coarse
    /// stage only — never a fabricated percentage.
    public func importMDX(url: URL) {
        conversionTask?.cancel()
        errorMessage = nil
        importSummary = nil
        importPhase = .preparing
        conversionTask = Task { [weak self] in
            await self?.performImport(url: url)
        }
    }

    /// Dismiss the post-import summary.
    public func dismissImportSummary() {
        importSummary = nil
    }

    private func performImport(url: URL, source: LibrarySourceDescriptor? = nil) async {
        do {
            let directory = try libraryDirectoryURL()
            let descriptor = try await sourceDescriptor(for: url, override: source)
            // Checked before converting: converting the same MDX twice is the
            // expensive half of the duplicate-import complaint.
            if let existing = LibraryStore.item(matching: descriptor, in: library),
               FileManager.default.fileExists(
                   atPath: LibraryStore.openTarget(for: existing, in: directory).path
               ) {
                importPhase = nil
                importSummary = "这本词典已在库中，已直接打开。"
                try await openExisting(existing, in: directory)
                return
            }
            let result = try await MDXImporter.convert(source: url) { [weak self] phase in
                Task { @MainActor in
                    self?.importPhase = phase
                }
            }
            guard !Task.isCancelled else { return }
            importPhase = nil
            importSummary = result.report.summary
            await performOpen(url: result.outputURL, source: descriptor)
        } catch is CancellationError {
            importPhase = nil
            return
        } catch {
            importPhase = nil
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Library

    /// Reopen the last dictionary on launch. Safe to call more than once.
    public func restore() {
        guard !hasRestored else { return }
        hasRestored = true
        guard documentURL == nil else { return }
        restoreTask = Task { [weak self] in
            await self?.performRestore()
        }
    }

    /// Wait for the restore started by `restore()`; no-op when none is running.
    func awaitRestore() async {
        await restoreTask?.value
    }

    /// Open a dictionary that is already in the library.
    public func open(item id: UUID) {
        guard let item = library.item(id: id), let directory = try? libraryDirectoryURL() else {
            return
        }
        importTask?.cancel()
        importTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await self.openExisting(item, in: directory)
            } catch is CancellationError {
                return
            } catch {
                self.errorMessage = error.localizedDescription
            }
        }
    }

    /// Remove a dictionary from the library and delete its local copy.
    public func removeItem(id: UUID) {
        guard let item = library.item(id: id), let directory = try? libraryDirectoryURL() else {
            return
        }
        DocumentLibrary.removeCopy(at: LibraryStore.copyDirectory(for: item, in: directory))
        if openItemID == id {
            importTask?.cancel()
            searchTask?.cancel()
            closeDictionary()
            openItemID = nil
            documentURL = nil
            documentName = nil
            query = ""
            hits = []
            selectedKey = nil
            entry = nil
        }
        LibraryStore.remove(id: id, from: &library)
        saveLibrary(in: directory)
    }

    /// Move duplicate copies into the library's `.Trash` folder.
    ///
    /// Nothing is deleted outright; the report names the folder so the copies
    /// can be recovered by hand.
    public func deduplicateExistingCopies() async -> LibraryDeduplicationReport? {
        guard let directory = try? libraryDirectoryURL() else { return nil }
        isDeduplicating = true
        defer { isDeduplicating = false }
        let start = library
        do {
            let outcome = try await Task.detached(priority: .utility) {
                var state = start
                let report = try LibraryStore.deduplicateExistingCopies(
                    in: directory,
                    state: &state
                )
                return (report, state)
            }.value
            let (report, state) = outcome
            guard !report.isEmpty else { return report }
            library = state
            if let openItemID, state.item(id: openItemID) == nil {
                closeDictionary()
                self.openItemID = nil
                documentURL = nil
                documentName = nil
                query = ""
                hits = []
                selectedKey = nil
                entry = nil
            }
            saveLibrary(in: directory)
            return report
        } catch {
            errorMessage = error.localizedDescription
            return nil
        }
    }

    private func performRestore() async {
        let directory: URL
        do {
            directory = try libraryDirectoryURL()
        } catch {
            libraryLog.warning("library unavailable: \(error.localizedDescription, privacy: .public)")
            return
        }
        var state = LibraryStore.load(from: directory)
        let pruned = LibraryStore.pruneMissing(in: &state, directory: directory)
        // Copies that predate `library.json` (older builds kept no index) are
        // registered so the reader sees them instead of an empty window.
        let adopted = LibraryStore.adoptableCopies(in: directory, registered: state)
        for item in adopted {
            LibraryStore.add(item, to: &state)
        }
        library = state
        if !pruned.isEmpty {
            libraryLog.warning("dropped \(pruned.count) missing librar\(pruned.count == 1 ? "y item" : "y items")")
        }
        if !pruned.isEmpty || !adopted.isEmpty {
            saveLibrary(in: directory)
        }
        if !adopted.isEmpty {
            libraryLog.info("adopted \(adopted.count) librar\(adopted.count == 1 ? "y copy" : "y copies")")
            // Nothing is opened behind the reader's back: the adopted entries
            // may be duplicates, so they pick the one to open.
            return
        }
        // A file opened while the restore was running wins.
        guard documentURL == nil,
              let id = state.lastOpenedID,
              let item = state.item(id: id) else { return }
        do {
            try await openExisting(item, in: directory)
        } catch is CancellationError {
            return
        } catch {
            // A copy that no longer opens is dropped quietly: one log line, no
            // alert on every launch.
            libraryLog.warning(
                "could not reopen \(item.displayName, privacy: .public): \(error.localizedDescription, privacy: .public)"
            )
            LibraryStore.remove(id: id, from: &library)
            saveLibrary(in: directory)
        }
    }

    /// Open `item` and mark it as the last opened dictionary.
    private func openExisting(_ item: LibraryItem, in directory: URL) async throws {
        let copy = LibraryStore.copyDirectory(for: item, in: directory)
        let opened = try OdcDictionary(url: copy)
        if Task.isCancelled {
            opened.close()
            return
        }
        closeDictionary()
        dictionary = opened
        documentURL = copy
        documentName = item.displayName
        openItemID = item.id
        query = ""
        hits = []
        selectedKey = nil
        entry = nil
        var state = library
        if state.item(id: item.id) == nil {
            LibraryStore.add(item, to: &state)
        }
        state.lastOpenedID = item.id
        do {
            try LibraryStore.save(state, to: directory)
            library = state
        } catch {
            libraryLog.error(
                "could not record the last opened dictionary: \(error.localizedDescription, privacy: .public)"
            )
            library = state
        }
    }

    private func register(_ item: LibraryItem, in directory: URL) throws {
        var state = library
        LibraryStore.add(item, to: &state)
        state.lastOpenedID = item.id
        try LibraryStore.save(state, to: directory)
        library = state
    }

    /// Drop an entry whose copy disappeared, so the next import copies again.
    private func forgetStaleEntry(_ item: LibraryItem, in directory: URL) {
        LibraryStore.remove(id: item.id, from: &library)
        saveLibrary(in: directory)
    }

    private func saveLibrary(in directory: URL) {
        do {
            try LibraryStore.save(library, to: directory)
        } catch {
            libraryLog.error("could not save the library: \(error.localizedDescription, privacy: .public)")
        }
    }

    private func libraryDirectoryURL() throws -> URL {
        if let container {
            return container
        }
        return try DocumentLibrary.applicationSupportContainer()
    }

    private func sourceDescriptor(
        for url: URL,
        override: LibrarySourceDescriptor?
    ) async throws -> LibrarySourceDescriptor {
        if let override {
            return override
        }
        return try await Task.detached(priority: .userInitiated) {
            try LibraryFileFingerprint.descriptor(for: url, accessingSecurityScope: true)
        }.value
    }

    private static func identifier(ofCopy url: URL) -> UUID {
        UUID(uuidString: url.deletingPathExtension().lastPathComponent) ?? UUID()
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

    /// Payload for an MDD resource key, `nil` when the dictionary has none.
    ///
    /// The entry reader calls this from the web view's scheme handler, so it
    /// must stay synchronous and cheap: the core reads one indexed block.
    public func resource(for key: String) -> Data? {
        guard let dictionary else { return nil }
        return try? dictionary.resource(for: key)
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
