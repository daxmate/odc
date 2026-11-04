import Foundation
import ODictCore
import Observation
import OSLog

/// Library diagnostics; stale entries are dropped quietly instead of via alerts.
private let libraryLog = Logger(subsystem: "com.daxmate.odict", category: "library")

/// One dictionary's share of the detail page.
///
/// A word can live in several open dictionaries at once; the detail page stacks
/// their entries on one scroll view, so each section carries its own name and
/// its own handle-backed entry.
public struct DictionaryEntrySection: Identifiable, Sendable, Equatable {
    /// Library identifier of the dictionary this section belongs to.
    public let dictionaryID: UUID
    /// Display name, copied so the section stays readable after the item is gone.
    public let name: String
    /// The dictionary's entry for the key on screen.
    public let entry: DictionaryEntry
    /// Note about how the entry resolved (a redirect target), `nil` when direct.
    public let notice: String?

    public var id: UUID { dictionaryID }

    public init(dictionaryID: UUID, name: String, entry: DictionaryEntry, notice: String?) {
        self.dictionaryID = dictionaryID
        self.name = name
        self.entry = entry
        self.notice = notice
    }
}

/// Observable state for the read-only dictionary UI.
@MainActor
@Observable
public final class DictionaryStore {
    /// Where the imported copy of the active dictionary lives (inside the app container).
    public var documentURL: URL? { activeDictionary?.url }
    /// Name of the file the reader picked, used for display.
    public var documentName: String? { activeDictionary?.name }
    /// Search text as typed; the debounced search follows it.
    public private(set) var query: String = ""
    /// Recent queries offered while the search field is empty.
    public let searchHistory: SearchHistoryStore
    /// Prefix-search results of the active dictionary, newest query only.
    public var hits: [SearchHit] {
        guard let activeDictionaryID else { return [] }
        return searchGroups.first { $0.dictionaryID == activeDictionaryID }?.hits ?? []
    }
    /// Shown when the working set deleted the entry the reader asked for.
    static let workingDeletedNotice = "该词条已在工作区删除（未提交）"

    /// Key of the entry currently shown.
    public private(set) var selectedKey: String?
    /// The entry for the current selection, once loaded.
    public private(set) var entry: DictionaryEntry?
    /// User-facing error text, cleared by `dismissError()`.
    public private(set) var errorMessage: String?
    /// Non-fatal note about the current entry (for example a redirect target
    /// the dictionary does not carry). The entry itself stays on screen.
    public private(set) var noticeMessage: String?
    /// Entries the startup migration could not move into the library folder.
    ///
    /// They stay usable: opening one reads the copy where it already lives.
    /// The library sheet lists them so a failed migration is never silent.
    public private(set) var migrationIssues: [LibraryStore.MigrationIssue] = []
    /// True while an import, a restore or a search is in flight.
    public private(set) var isLoading = false
    /// Conversion phase of the running MDX import, if any.
    public private(set) var importPhase: MDXImportPhase?
    public private(set) var importProgress: MDXImportProgress?
    /// Summary shown once an MDX conversion finished, until dismissed.
    public private(set) var importSummary: String?
    /// First-use trust prompt for a signed import or update, if one is open.
    public private(set) var pendingTrustRequest: PendingTrustRequest?
    /// Unsigned patch the reader asked to apply; shown until acknowledged.
    public private(set) var pendingUnsignedUpdate: PendingUnsignedUpdate?
    /// Dictionaries copied into the app's storage.
    public private(set) var library = LibraryState()
    /// Bookmarks and recent entries, mirrored from `reader.json`.
    public private(set) var reader = ReaderState()
    public private(set) var isDeduplicating = false
    /// Dictionaries that are open right now, in the order they were opened.
    public private(set) var openDictionaries: [OpenDictionary] = []
    /// Entries for the key on screen, one per open dictionary, active first.
    ///
    /// The detail page draws these as sections, so a word carried by two
    /// dictionaries reads on one page instead of one dictionary at a time.
    public private(set) var entrySections: [DictionaryEntrySection] = []
    /// Results of the last search, one group per dictionary that answered.
    ///
    /// Groups follow the order in `openDictionaries`, so the sidebar does not
    /// reshuffle when a slow dictionary answers after a fast one.
    public private(set) var searchGroups: [DictionarySearchGroup] = []
    /// The dictionary `hits`, `selectedKey` and `entry` belong to.
    public private(set) var activeDictionaryID: UUID?
    /// Bumped whenever a new entry lands on screen. The compact layout watches
    /// it instead of `selectedKey`, so picking the same key from another
    /// dictionary still pushes the detail column.
    public private(set) var selectionRevision = 0

    /// More dictionaries than this and the sidebar stops being a sidebar.
    public static let maximumOpenDictionaries = 8
    /// Hits asked from each open dictionary for one query.
    static let hitsPerDictionary = 20

    /// One handle per open dictionary, keyed by its library identifier.
    @ObservationIgnored private var handles: [UUID: OdcDictionary] = [:]
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var importTask: Task<Void, Never>?
    @ObservationIgnored private var conversionTask: Task<Void, Never>?
    @ObservationIgnored private var restoreTask: Task<Void, Never>?
    @ObservationIgnored private var updateTask: Task<Void, Never>?
    /// Resumes the import or update flow once the trust alert is answered.
    @ObservationIgnored private var trustContinuation: CheckedContinuation<Bool, Never>?
    @ObservationIgnored private var summaryTask: Task<Void, Never>?
    /// Stamped on every summary change so a stale timer cannot clear a newer one.
    @ObservationIgnored private var summaryGeneration = 0
    /// How long a success note stays on screen.
    @ObservationIgnored private let importSummaryLifetime: Duration
    @ObservationIgnored private var hasRestored = false
    /// True while `performRestore` opens the saved set: writes are deferred so
    /// the whole restore lands as a single save.
    @ObservationIgnored private var isRestoringOpenSet = false
    /// Directory `reader` was loaded from.
    @ObservationIgnored private var readerDirectory: URL?
    /// Debounced write of `session.json`; cancelled and flushed on demand.
    @ObservationIgnored private var sessionSaveTask: Task<Void, Never>?
    /// True while the stored session is being applied, so restoring does not
    /// count as the reader opening the entry again.
    @ObservationIgnored private var isRestoringSession = false
    private let container: URL?
    /// Debounce before a keystroke turns into a search.
    private static let debounceMilliseconds = 200

    /// `container` overrides the library folder; tests point it at a temporary
    /// directory.
    ///
    /// `importSummaryLifetime` describes the import that just finished, so it
    ///     clears itself instead of sitting under the entry until someone
    ///     dismisses it.
    public init(
        container: URL? = nil,
        importSummaryLifetime: Duration = .seconds(6),
        searchHistory: SearchHistoryStore? = nil
    ) {
        self.container = container
        self.importSummaryLifetime = importSummaryLifetime
        self.searchHistory = searchHistory ?? SearchHistoryStore()
    }

    /// The open dictionary the reader is looking at, if any.
    public var activeDictionary: OpenDictionary? {
        guard let activeDictionaryID else { return nil }
        return openDictionaries.first { $0.id == activeDictionaryID }
    }

    /// The handle behind `activeDictionary`.
    private var activeHandle: OdcDictionary? {
        guard let activeDictionaryID else { return nil }
        return handles[activeDictionaryID]
    }

    /// True while the library item `id` has an open handle.
    public func isOpen(_ id: UUID) -> Bool { handles[id] != nil }

    /// True when `id` is the dictionary the detail column follows.
    public func isActive(_ id: UUID) -> Bool { activeDictionaryID == id }

    // MARK: - Editing

    /// Directory-backed dictionaries are editable; single-file packs are not.
    ///
    /// Records imported before `sourceIsDirectory` existed have no flag; treat
    /// those as directories so editing keeps working for existing libraries.
    public var isEditingAvailable: Bool {
        guard let id = activeDictionaryID, handles[id] != nil,
              let item = library.item(id: id) else { return false }
        return item.sourceIsDirectory ?? true
    }

    /// A fresh editor bound to the active dictionary, or `nil` when none is open.
    public func makeEditorModel() -> DictionaryEditorModel? {
        guard let id = activeDictionaryID, let handle = handles[id] else { return nil }
        return DictionaryEditorModel(editing: handle) { [weak self] in
            self?.refreshAfterEditing()
        }
    }

    /// A CSV importer that writes into the active dictionary's working set.
    public func makeCSVImportModel() -> CSVImportModel? {
        guard let handle = activeHandle else { return nil }
        return CSVImportModel(editing: handle, libraryKey: activeDictionaryID?.uuidString ?? "")
    }

    /// Re-run the reader's query after the editor changed the working set.
    public func refreshAfterEditing() {
        if query.isEmpty {
            searchGroups = []
        } else {
            setQuery(query)
        }
        reloadSelectedEntry()
        refreshEntrySections()
    }

    /// Pick up a save without touching search history or the query.
    private func reloadSelectedEntry() {
        guard let key = selectedKey, let dictionary = activeHandle else { return }
        switch try? dictionary.resolvePreferWorking(key) {
        case let .entry(loaded, _, _)?:
            entry = loaded
            noticeMessage = nil
        case .deleted?:
            entry = nil
            noticeMessage = Self.workingDeletedNotice
        default:
            break
        }
    }

    // MARK: - Opening and closing

    /// Import `url` into the sandbox and open the copy.
    /// Import `url` into the sandbox and open the copy.
    ///
    /// The dictionary joins the open ones instead of replacing them, and the
    /// new one becomes active.
    public func open(url: URL) {
        importTask?.cancel()
        importTask = Task { [weak self] in
            await self?.performOpen(url: url)
        }
    }

    /// Close the active dictionary and delete its imported copy.
    ///
    /// Another open dictionary takes its place; the store only falls back to
    /// the empty state when nothing else is open.
    public func close() {
        importTask?.cancel()
        conversionTask?.cancel()
        searchTask?.cancel()
        guard let active = activeDictionary else {
            resetEverything()
            return
        }
        let directory = try? libraryDirectoryURL()
        DocumentLibrary.removeCopy(at: active.url)
        // Close first so the saved open set no longer lists the deleted copy.
        closeLibraryItem(id: active.id)
        forget(active.id, in: directory)
        errorMessage = nil
        isLoading = false
        importPhase = nil
        clearImportSummary()
    }

    /// Close `id` and leave its copy in the library, ready to open again.
    public func closeLibraryItem(id: UUID) {
        guard handles[id] != nil else { return }
        if activeDictionaryID == id {
            importTask?.cancel()
            searchTask?.cancel()
        }
        handles.removeValue(forKey: id)?.close()
        openDictionaries.removeAll { $0.id == id }
        searchGroups.removeAll { $0.dictionaryID == id }
        if activeDictionaryID == id {
            activeDictionaryID = openDictionaries.last?.id
            resetSelection()
            if activeDictionaryID == nil {
                query = ""
                isLoading = false
            }
        }
        persistOpenState(in: try? libraryDirectoryURL())
        refreshEntrySections()
    }

    /// Make `id` the dictionary the detail column follows.
    ///
    /// The query and every open dictionary's hits stay put; only the selection
    /// moves, because the entry belonged to the previous dictionary.
    public func activateDictionary(id: UUID) {
        guard handles[id] != nil, activeDictionaryID != id else { return }
        activeDictionaryID = id
        resetSelection()
        persistOpenState(in: try? libraryDirectoryURL())
        scheduleSessionSave()
    }

    /// Open `item` the first time, or bring it forward when it is already open.
    @discardableResult
    private func openExisting(_ item: LibraryItem, in directory: URL) async throws -> Bool {
        if handles[item.id] != nil {
            activateDictionary(id: item.id)
            return true
        }
        guard canOpenAnotherDictionary else {
            presentDictionaryLimit()
            return false
        }
        let copy = LibraryStore.copyDirectory(for: item, in: directory)
        let opened = try OdcDictionary(url: copy)
        if Task.isCancelled {
            opened.close()
            return false
        }
        adopt(opened, as: item, at: copy)
        persistOpenState(in: directory)
        return true
    }

    /// Put a freshly opened handle on the list and make it active.
    private func adopt(_ opened: OdcDictionary, as item: LibraryItem, at url: URL) {
        handles[item.id] = opened
        let entry = OpenDictionary(id: item.id, name: item.displayName, url: url)
        if let index = openDictionaries.firstIndex(where: { $0.id == item.id }) {
            openDictionaries[index] = entry
        } else {
            openDictionaries.append(entry)
        }
        activeDictionaryID = item.id
        query = ""
        searchGroups = []
        resetSelection()
    }

    /// True while one more dictionary fits under `maximumOpenDictionaries`.
    private var canOpenAnotherDictionary: Bool {
        openDictionaries.count < Self.maximumOpenDictionaries
    }

    private func presentDictionaryLimit() {
        errorMessage = "最多同时打开 \(Self.maximumOpenDictionaries) 本词典，请先关闭一本。"
    }

    /// Record the open dictionaries and the active one, then write `library.json`.
    ///
    /// Called after every open, close and activation so the next launch can
    /// bring the same set back. Restore suppresses it and saves once instead.
    private func persistOpenState(in directory: URL?) {
        guard !isRestoringOpenSet, let directory else { return }
        var state = library
        state.openItemIDs.removeAll()
        for dictionary in openDictionaries {
            LibraryStore.markOpen(dictionary.id, in: &state, limit: Self.maximumOpenDictionaries)
        }
        state.lastOpenedID = activeDictionaryID
        do {
            try LibraryStore.save(state, to: directory)
        } catch {
            libraryLog.error(
                "could not record the open dictionaries: \(error.localizedDescription, privacy: .public)"
            )
        }
        library = state
    }

    /// Drop a dictionary from the library and from the reader's memory.
    private func forget(_ id: UUID, in directory: URL?) {
        guard let directory else { return }
        LibraryStore.remove(id: id, from: &library)
        saveLibrary(in: directory)
        updateReader(in: directory) {
            ReaderStore.forget(dictionaryID: id.uuidString, in: &$0)
        }
    }

    /// Nothing is on screen: the state a fresh launch starts from.
    private func resetSelection() {
        selectedKey = nil
        entry = nil
        noticeMessage = nil
        entrySections = []
    }

    private func resetEverything() {
        query = ""
        searchGroups = []
        resetSelection()
        errorMessage = nil
        isLoading = false
        scheduleSessionSave()
        importPhase = nil
        clearImportSummary()
    }

    private func performOpen(url: URL, source: LibrarySourceDescriptor? = nil) async {
        errorMessage = nil
        isLoading = true
        defer { isLoading = false }

        do {
            let directory = try libraryDirectoryURL()
            let descriptor = try await sourceDescriptor(for: url, override: source)
            let isDirectory = (try? await Task.detached(priority: .userInitiated) {
                try LibraryFileFingerprint.isDirectorySource(url, accessingSecurityScope: true)
            }.value) ?? false
            if Task.isCancelled {
                return
            }

            // The same file already imported: reopen the copy instead of
            // paying for a second 200 MB duplicate.
            if let existing = LibraryStore.item(matching: descriptor, in: library) {
                if FileManager.default.fileExists(
                    atPath: LibraryStore.openTarget(for: existing, in: directory).path
                ) {
                    if try await openExisting(existing, in: directory) {
                        presentImportSummary("这本词典已在库中，已直接打开。")
                    }
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
                if try await openExisting(orphan, in: directory) {
                    presentImportSummary("这本词典已在库中，已直接打开。")
                }
                return
            }

            guard canOpenAnotherDictionary else {
                presentDictionaryLimit()
                return
            }
            let copy = try await Task.detached(priority: .userInitiated) {
                try DocumentLibrary.copyDocument(at: url, into: directory)
            }.value
            if Task.isCancelled {
                DocumentLibrary.removeCopy(at: copy)
                return
            }

            // A signature next to the picked file gates the import: valid and
            // trusted goes through, a first-use key asks once, everything else
            // is refused before the copy is registered. The picker's security
            // scope is needed again to read the sidecar.
            let scopedSignature = url.startAccessingSecurityScopedResource()
            defer {
                if scopedSignature {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            let signatureURL = ReleaseSidecar.signatureURL(for: url)
            var signatureInfo: LibrarySignatureInfo?
            var showsUnsignedNotice = false
            if ReleaseSidecar.hasSignature(for: url) {
                let verification = try ODCPublishing.verifySignatureFile(
                    path: url.path,
                    sigPath: signatureURL.path
                )
                let publishers = try ODCPublishing.publishers(repoPath: copy.path)
                switch TrustDecision.evaluate(verification: verification, publishers: publishers) {
                case .trusted:
                    signatureInfo = Self.signatureInfo(for: verification)
                case .firstTrust:
                    let key = verification.publicKeyHex ?? ""
                    let approved = await requestTrust(
                        PendingTrustRequest(
                            signerName: verification.displayName,
                            fingerprint: verification.fingerprint ?? "（无法计算）",
                            publicKeyHex: key,
                            action: .importCopy(
                                repoPath: copy.path,
                                copyURL: copy,
                                sourceName: descriptor.name,
                                sourceBytes: descriptor.bytes,
                                sourceHash: descriptor.contentHash,
                                sourceIsDirectory: isDirectory
                            )
                        )
                    )
                    guard approved else {
                        DocumentLibrary.removeCopy(at: copy)
                        return
                    }
                    _ = try ODCPublishing.trustAdd(
                        repoPath: copy.path,
                        publicKeyHex: key,
                        name: verification.signer
                    )
                    signatureInfo = Self.signatureInfo(for: verification)
                case .unknownPublisher(let key):
                    DocumentLibrary.removeCopy(at: copy)
                    throw OdcError.verifyFailed(
                        "未知发布者 \(Self.fingerprint(for: key))；请先在「信任管理」中添加这个公钥。"
                    )
                case .invalidSignature(let text):
                    DocumentLibrary.removeCopy(at: copy)
                    throw OdcError.verifyFailed(text)
                case .missingPublicKey:
                    DocumentLibrary.removeCopy(at: copy)
                    throw OdcError.verifyFailed("签名缺少公钥，无法判断发布者。")
                }
            } else {
                signatureInfo = LibrarySignatureInfo(signed: false)
                showsUnsignedNotice = true
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
                sourceHash: descriptor.contentHash,
                sourceIsDirectory: isDirectory,
                signature: signatureInfo
            )
            do {
                // Register only once the copy opens, and drop the copy again if
                // the registration cannot be saved: no half-recorded imports.
                try register(item, in: directory)
            } catch {
                opened.close()
                DocumentLibrary.removeCopy(at: copy)
                throw error
            }
            adopt(opened, as: item, at: copy)
            persistOpenState(in: directory)
            if showsUnsignedNotice {
                announceUnsignedImport()
            }
        } catch is CancellationError {
            return
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Signatures and updates

    /// Accept the first-use publisher and resume the waiting flow.
    public func confirmPendingTrust() {
        resumeTrustPrompt(true)
    }

    /// Refuse the first-use publisher; the waiting flow drops its copy.
    public func cancelPendingTrust() {
        resumeTrustPrompt(false)
    }

    /// Dismiss the explanation shown for an unsigned patch.
    public func dismissPendingUnsignedUpdate() {
        pendingUnsignedUpdate = nil
    }

    /// Apply the patch at `patchURL` to the library item `itemID`.
    ///
    /// The patch is signature-gated exactly like an import: valid and trusted
    /// applies, a first-use key asks once, an unknown key is refused. A patch
    /// without a sidecar is refused too: the FFI only exposes the signed apply
    /// entry point, so the repository is never touched with unverified bytes.
    public func applyUpdate(itemID: UUID, patchURL: URL) {
        updateTask?.cancel()
        updateTask = Task { [weak self] in
            await self?.performUpdate(itemID: itemID, patchURL: patchURL)
        }
    }

    /// Repository path backing a library item, for the trust sheet.
    public func repoPath(for itemID: UUID) -> String? {
        guard let item = library.item(id: itemID),
              let directory = try? libraryDirectoryURL() else {
            return nil
        }
        return LibraryStore.copyDirectory(for: item, in: directory).path
    }

    private func performUpdate(itemID: UUID, patchURL: URL) async {
        errorMessage = nil
        do {
            let directory = try libraryDirectoryURL()
            guard let item = library.item(id: itemID) else {
                throw OdcError.patchFailed("这本词典不在库中。")
            }
            let repoPath = LibraryStore.copyDirectory(for: item, in: directory).path
            let scopedPatch = patchURL.startAccessingSecurityScopedResource()
            defer {
                if scopedPatch {
                    patchURL.stopAccessingSecurityScopedResource()
                }
            }
            let sigURL = ReleaseSidecar.signatureURL(for: patchURL)
            guard ReleaseSidecar.hasSignature(for: patchURL) else {
                pendingUnsignedUpdate = PendingUnsignedUpdate(itemID: itemID, patchURL: patchURL)
                return
            }
            let verification = try ODCPublishing.verifySignatureFile(
                path: patchURL.path,
                sigPath: sigURL.path
            )
            let publishers = try ODCPublishing.publishers(repoPath: repoPath)
            switch TrustDecision.evaluate(verification: verification, publishers: publishers) {
            case .trusted:
                break
            case .firstTrust:
                let key = verification.publicKeyHex ?? ""
                let approved = await requestTrust(
                    PendingTrustRequest(
                        signerName: verification.displayName,
                        fingerprint: verification.fingerprint ?? "（无法计算）",
                        publicKeyHex: key,
                        action: .applyPatch(
                            itemID: itemID,
                            patchURL: patchURL,
                            repoPath: repoPath,
                            sigPath: sigURL.path
                        )
                    )
                )
                guard approved else { return }
                _ = try ODCPublishing.trustAdd(
                    repoPath: repoPath,
                    publicKeyHex: key,
                    name: verification.signer
                )
            case .unknownPublisher(let key):
                throw OdcError.verifyFailed(
                    "未知发布者 \(Self.fingerprint(for: key))；请先在「信任管理」中添加这个公钥。"
                )
            case .invalidSignature(let text):
                throw OdcError.verifyFailed(text)
            case .missingPublicKey:
                throw OdcError.verifyFailed("签名缺少公钥，无法判断发布者。")
            }
            try await applyVerifiedPatch(
                itemID: itemID,
                patchPath: patchURL.path,
                sigPath: sigURL.path,
                repoPath: repoPath,
                verification: verification
            )
        } catch {
            present(error: error)
        }
    }

    private func applyVerifiedPatch(
        itemID: UUID,
        patchPath: String,
        sigPath: String,
        repoPath: String,
        verification: SignatureVerification
    ) async throws {
        let report = try await Task.detached(priority: .userInitiated) {
            try ODCPublishing.applySignedPatch(
                repoPath: repoPath,
                patchPath: patchPath,
                sigPath: sigPath
            )
        }.value
        if let index = library.items.firstIndex(where: { $0.id == itemID }) {
            library.items[index].signature = Self.signatureInfo(for: verification)
            if let directory = try? libraryDirectoryURL() {
                saveLibrary(in: directory)
            }
        }
        presentImportSummary(report.summaryText)
        if isOpen(itemID) {
            refreshAfterEditing()
        }
    }

    /// Wait for the reader to answer the first-use trust alert.
    private func requestTrust(_ request: PendingTrustRequest) async -> Bool {
        pendingTrustRequest = request
        return await withCheckedContinuation { continuation in
            trustContinuation = continuation
        }
    }

    private func resumeTrustPrompt(_ approved: Bool) {
        guard let continuation = trustContinuation else { return }
        trustContinuation = nil
        pendingTrustRequest = nil
        continuation.resume(returning: approved)
    }

    private static func signatureInfo(for verification: SignatureVerification) -> LibrarySignatureInfo {
        LibrarySignatureInfo(
            signed: true,
            publisherName: verification.signer,
            publicKeyHex: verification.publicKeyHex,
            verifiedAt: Date()
        )
    }

    private static func fingerprint(for publicKeyHex: String) -> String {
        let normalised = PublisherIdentity.normalizedPublicKeyHex(publicKeyHex)
        guard PublisherIdentity.isValidPublicKeyHex(normalised) else { return publicKeyHex }
        return PublisherIdentity(name: "", publicKeyHex: normalised).fingerprint
    }

    private static let unsignedNoticeKey = "ODCUnsignedImportNoticeShown"

    /// Explain once, on the first unsigned import, that no publisher could be verified.
    private func announceUnsignedImport() {
        let defaults = UserDefaults.standard
        guard !defaults.bool(forKey: Self.unsignedNoticeKey) else { return }
        defaults.set(true, forKey: Self.unsignedNoticeKey)
        presentImportSummary("这是一本未签名的词典：导入成功，但无法验证发布者。")
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
        clearImportSummary()
        importPhase = .preparing
        importProgress = .preparingSample
        conversionTask = Task { [weak self] in
            await self?.performImport(url: url)
        }
    }

    /// Dismiss the post-import summary right away.
    public func dismissImportSummary() {
        clearImportSummary()
    }

    /// Shows the note for the import that just finished.
    ///
    /// A success note is transient: it clears itself after
    /// `importSummaryLifetime` unless the reader closes it first. Failures go
    /// through `errorMessage` and stay until dismissed.
    func presentImportSummary(_ text: String) {
        summaryGeneration += 1
        let generation = summaryGeneration
        importSummary = text
        summaryTask?.cancel()
        let lifetime = importSummaryLifetime
        summaryTask = Task { [weak self] in
            try? await Task.sleep(for: lifetime)
            guard !Task.isCancelled, let self, self.summaryGeneration == generation else { return }
            self.importSummary = nil
            self.summaryTask = nil
        }
    }

    private func clearImportSummary() {
        summaryGeneration += 1
        summaryTask?.cancel()
        summaryTask = nil
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
                importProgress = nil
                if try await openExisting(existing, in: directory) {
                    presentImportSummary("这本词典已在库中，已直接打开。")
                }
                return
            }
            // The conversion is a staging artefact: it lands in the library's
            // own folder, never next to the source the reader picked. It is a
            // directory repository, so the copy registered below is editable.
            let staging = directory.appending(path: "Imports", directoryHint: .isDirectory)
            try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
            let result = try await MDXImporter.convertToRepo(
                source: url,
                documents: staging,
                onProgress: { [weak self] sample in
                    Task { @MainActor in
                        self?.importProgress = sample
                        self?.importPhase = sample.phase
                    }
                }
            )
            guard !Task.isCancelled else { return }
            importPhase = nil
            importProgress = nil
            presentImportSummary(result.report.summary)
            await performOpen(url: result.outputURL, source: descriptor)
            discardStagedConversion(result.outputURL, in: staging)
        } catch is CancellationError {
            importPhase = nil
            importProgress = nil
            return
        } catch {
            importPhase = nil
            importProgress = nil
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Pack conversion

    /// Turn a read-only pack into an editable directory copy.
    ///
    /// The pack stays in the library untouched; the copy is a new item with
    /// its own UUID and its own `odc.db`. Opening the copy afterwards makes the
    /// editor available straight away.
    public func convertPackToEditable(itemID: UUID) async {
        errorMessage = nil
        guard let item = library.item(id: itemID) else {
            errorMessage = "找不到这本词典。"
            return
        }
        guard item.sourceIsDirectory == false else {
            errorMessage = "这本词典已经是可编辑的。"
            return
        }
        do {
            let directory = try libraryDirectoryURL()
            let source = LibraryStore.openTarget(for: item, in: directory)
            let copyID = UUID()
            let storedName = "\(copyID.uuidString).odc"
            let copyURL = LibraryStore.copyDirectory(
                for: LibraryItem(
                    id: copyID,
                    displayName: item.displayName,
                    storedName: storedName,
                    sourceName: item.sourceName,
                    sourceBytes: item.sourceBytes
                ),
                in: directory
            )
            try await Task.detached(priority: .userInitiated) {
                try MDXImporter.copyPackToRepo(pack: source, repoDirectory: copyURL)
            }.value
            let descriptor = try await Task.detached(priority: .userInitiated) {
                try LibraryFileFingerprint.descriptor(
                    for: copyURL,
                    accessingSecurityScope: false
                )
            }.value
            let copy = LibraryItem(
                id: copyID,
                displayName: item.displayName,
                storedName: storedName,
                sourceName: descriptor.name,
                sourceBytes: descriptor.bytes,
                sourceHash: descriptor.contentHash,
                sourceIsDirectory: true
            )
            do {
                try register(copy, in: directory)
            } catch {
                DocumentLibrary.removeCopy(at: copyURL)
                throw error
            }
            do {
                _ = try await openExisting(copy, in: directory)
            } catch {
                forget(copy.id, in: directory)
                DocumentLibrary.removeCopy(at: copyURL)
                throw error
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Library

    /// Reopen the dictionaries that were open on the last launch, oldest
    /// first, and put the one that was active back in front.
    ///
    /// Safe to call more than once; a copy that no longer opens is skipped and
    /// dropped instead of blocking the launch.
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
    /// Remove a library item and delete its copy.
    public func removeItem(id: UUID) {
        guard let item = library.item(id: id), let directory = try? libraryDirectoryURL() else {
            return
        }
        DocumentLibrary.removeCopy(at: LibraryStore.copyDirectory(for: item, in: directory))
        if handles[id] != nil {
            importTask?.cancel()
            closeLibraryItem(id: id)
        }
        LibraryStore.remove(id: id, from: &library)
        saveLibrary(in: directory)
        updateReader(in: directory) {
            ReaderStore.forget(dictionaryID: id.uuidString, in: &$0)
        }
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
            for entry in openDictionaries where state.item(id: entry.id) == nil {
                closeLibraryItem(id: entry.id)
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
        // Copies still living outside the library folder move in first, so the
        // prune below sees them where they will stay. A failed move keeps the
        // entry and is reported in the library sheet instead of blocking the
        // launch (an iCloud placeholder is the usual reason).
        let migration = LibraryStore.migrateExternalCopies(in: directory, state: &state) { url in
            let opened = try OdcDictionary(url: url)
            opened.close()
        }
        if !migration.migrated.isEmpty {
            libraryLog.info(
                "migrated \(migration.migrated.count) dictionar\(migration.migrated.count == 1 ? "y" : "ies") into the library"
            )
        }
        for issue in migration.issues {
            libraryLog.warning(
                "could not migrate \(issue.displayName, privacy: .public): \(issue.reason, privacy: .public)"
            )
        }
        let pruned = LibraryStore.pruneMissing(in: &state, directory: directory)
        // A failed move keeps its entry; the prune above still drops entries
        // whose copy is gone for good, so those are not reported twice.
        migrationIssues = migration.issues.filter { !pruned.contains($0.id) }
        // Copies that predate `library.json` (older builds kept no index) are
        // registered so the reader sees them instead of an empty window.
        let adopted = LibraryStore.adoptableCopies(in: directory, registered: state)
        for item in adopted {
            LibraryStore.add(item, to: &state)
        }
        // Files written before multi-dictionary support only carry
        // `lastOpenedID`; `restoreOpenIDs` falls back to it and drops ids whose
        // entry is gone, so the loop below never chases a stale copy.
        let storedOpenIDs = state.openItemIDs
        let openIDs = LibraryStore.restoreOpenIDs(
            in: &state,
            limit: Self.maximumOpenDictionaries
        )
        let openSetCleaned = state.openItemIDs != storedOpenIDs
        library = state
        loadReader(in: directory)
        if !pruned.isEmpty {
            libraryLog.warning("dropped \(pruned.count) missing librar\(pruned.count == 1 ? "y item" : "y items")")
        }
        var needsSave = !pruned.isEmpty
            || !adopted.isEmpty
            || !migration.migrated.isEmpty
            || openSetCleaned
        if !adopted.isEmpty {
            libraryLog.info("adopted \(adopted.count) librar\(adopted.count == 1 ? "y copy" : "y copies")")
            // Nothing is opened behind the reader's back: the adopted entries
            // may be duplicates, so they pick the one to open.
            if needsSave {
                saveLibrary(in: directory)
            }
            return
        }
        // A file opened while the restore was running wins.
        guard documentURL == nil, !openIDs.isEmpty else {
            if needsSave {
                saveLibrary(in: directory)
            }
            return
        }
        // Reopen the saved set oldest first, writing nothing until the end so
        // the whole restore lands as one save.
        var restoredIDs: [UUID] = []
        isRestoringOpenSet = true
        for id in openIDs {
            // A dictionary opened while the restore was running wins over the
            // saved set; only the ids being restored count as ours.
            let untouched = openDictionaries.allSatisfy { openIDs.contains($0.id) }
            guard untouched, let item = library.item(id: id) else { break }
            do {
                try await openExisting(item, in: directory)
                restoredIDs.append(id)
            } catch is CancellationError {
                isRestoringOpenSet = false
                return
            } catch {
                // A copy that no longer opens is dropped quietly: one log line,
                // no alert on every launch.
                libraryLog.warning(
                    "could not reopen \(item.displayName, privacy: .public): \(error.localizedDescription, privacy: .public)"
                )
                LibraryStore.remove(id: id, from: &library)
                needsSave = true
            }
        }
        // Prefer the dictionary that was active; otherwise the last one that
        // opened stays in front.
        if let wanted = state.lastOpenedID, handles[wanted] != nil {
            activateDictionary(id: wanted)
        }
        isRestoringOpenSet = false
        if needsSave || restoredIDs.count != openIDs.count {
            persistOpenState(in: directory)
        }
        await restoreSession(in: directory)
    }

    /// Open `item` and mark it as the last opened dictionary.

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
        updateReader(in: directory) {
            ReaderStore.forget(dictionaryID: item.id.uuidString, in: &$0)
        }
    }

    private func saveLibrary(in directory: URL) {
        do {
            try LibraryStore.save(library, to: directory)
        } catch {
            libraryLog.error("could not save the library: \(error.localizedDescription, privacy: .public)")
        }
    }

    // MARK: - Session

    /// Write the pending session now; the normal path is debounced.
    public func flushSession() {
        sessionSaveTask?.cancel()
        sessionSaveTask = nil
        guard let directory = try? libraryDirectoryURL() else { return }
        writeSession(in: directory)
    }

    private func scheduleSessionSave() {
        guard let directory = try? libraryDirectoryURL() else { return }
        sessionSaveTask?.cancel()
        sessionSaveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            self?.writeSession(in: directory)
        }
    }

    private func writeSession(in directory: URL) {
        let session = ReaderSession(
            dictionaryID: activeDictionaryID?.uuidString,
            key: selectedKey,
            query: query
        )
        do {
            try SessionStore.save(session, in: directory)
        } catch {
            libraryLog.error(
                "could not save the session: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    /// Apply `session.json` once the library has opened. Anything that no
    /// longer exists is skipped in silence.
    private func restoreSession(in directory: URL) async {
        let session = SessionStore.load(in: directory)
        guard let plan = SessionRestore.plan(for: session, using: self) else { return }
        isRestoringSession = true
        defer { isRestoringSession = false }
        if activeDictionaryID != plan.dictionaryID {
            activateDictionary(id: plan.dictionaryID)
        }
        if let storedQuery = plan.query {
            setQuery(storedQuery)
            // Let the debounced search land before the entry is selected, so a
            // late result cannot clear the restored selection.
            let search = searchTask
            await search?.value
        }
        if let key = plan.key {
            await select(key: key)
        }
    }

    // MARK: - Bookmarks and history

    /// Entries opened most recently, newest first.
    public var recentEntries: [EntryRef] { reader.history }

    /// Bookmarked entries, newest first.
    public var bookmarkedEntries: [EntryRef] { reader.bookmarks }

    /// Whether `key` is bookmarked in the dictionary that is open right now.
    public func isBookmarked(key: String) -> Bool {
        guard let dictionaryID = activeDictionaryID?.uuidString else { return false }
        return reader.bookmarks.contains {
            $0.dictionaryID == dictionaryID && $0.key == key
        }
    }

    /// Adds the entry on screen to the bookmarks, or removes it again.
    public func toggleBookmark() {
        guard let entry,
              let dictionaryID = activeDictionaryID?.uuidString,
              let directory = try? libraryDirectoryURL()
        else { return }
        let ref = EntryRef(dictionaryID: dictionaryID, key: entry.key, headword: entry.display)
        updateReader(in: directory) { ReaderStore.toggleBookmark(ref, in: &$0) }
    }

    /// Opens a remembered entry, switching dictionaries first when needed.
    public func openEntry(_ ref: EntryRef) async {
        if let id = UUID(uuidString: ref.dictionaryID), id != activeDictionaryID {
            guard let item = library.item(id: id), let directory = try? libraryDirectoryURL() else {
                noticeMessage = "这本词典已不在词典库中。"
                return
            }
            do {
                try await openExisting(item, in: directory)
            } catch is CancellationError {
                return
            } catch {
                errorMessage = error.localizedDescription
                return
            }
            // The open can be refused (the limit is reached, the copy is gone);
            // selecting now would land in whichever dictionary is still active.
            guard activeDictionaryID == id else { return }
        }
        await select(key: ref.key)
    }

    /// Loads `reader.json` once per storage folder.
    private func loadReader(in directory: URL) {
        guard readerDirectory != directory else { return }
        readerDirectory = directory
        let result = ReaderStore.loadResult(in: directory)
        reader = result.state
        if result.recoveredFromCorruptFile {
            noticeMessage = "阅读记录文件已损坏，已重置。"
        }
    }

    /// Applies `change` to the reader state and writes it back right away.
    private func updateReader(in directory: URL, _ change: (inout ReaderState) -> Void) {
        loadReader(in: directory)
        change(&reader)
        do {
            try ReaderStore.save(reader, in: directory)
        } catch {
            libraryLog.error("could not save the reader state: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Remembers an entry that resolved and is now on screen.
    private func recordHistory(for entry: DictionaryEntry) {
        guard let dictionaryID = activeDictionaryID?.uuidString,
              let directory = try? libraryDirectoryURL()
        else { return }
        let ref = EntryRef(dictionaryID: dictionaryID, key: entry.key, headword: entry.display)
        updateReader(in: directory) { ReaderStore.recordHistory(ref, in: &$0) }
    }

    /// Delete a conversion that only existed to be copied into the library.
    ///
    /// Anything outside `staging` (a `.odc` the reader picked, or a file an
    /// older build left next to its source) is left alone.
    private func discardStagedConversion(_ url: URL, in staging: URL) {
        let root = staging.standardizedFileURL.path
        let target = url.standardizedFileURL.path
        let prefix = root.hasSuffix("/") ? root : root + "/"
        guard target.hasPrefix(prefix) else { return }
        try? FileManager.default.removeItem(at: url)
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

    /// Update the search text and schedule a debounced search in every open
    /// dictionary.
    public func setQuery(_ text: String) {
        query = text
        scheduleSessionSave()
        searchTask?.cancel()

        guard activeDictionaryID != nil else {
            searchGroups = []
            return
        }

        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            searchGroups = []
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

    /// Ask every open dictionary, in opening order, for at most
    /// `hitsPerDictionary` hits.
    ///
    /// A dictionary that fails (a stale handle, a copy removed behind our back)
    /// is skipped and logged; the other groups still publish.
    private func runSearch(_ text: String) async {
        let open = openDictionaries
        guard !open.isEmpty else { return }
        isLoading = true
        defer { isLoading = false }

        var groups: [DictionarySearchGroup] = []
        for entry in open {
            guard !Task.isCancelled else { return }
            guard let handle = handles[entry.id] else { continue }
            do {
                // Prefix hits win; substring search only fills the empty case,
                // so a dictionary never mixes the two orderings.
                var results = try handle.search(text, limit: Self.hitsPerDictionary)
                var matchKind = SearchMatchKind.prefix
                if results.isEmpty {
                    results = try handle.searchSubstring(text, limit: Self.hitsPerDictionary)
                    matchKind = .substring
                }
                groups.append(
                    DictionarySearchGroup(
                        dictionaryID: entry.id,
                        name: entry.name,
                        hits: results,
                        matchKind: matchKind
                    )
                )
            } catch {
                libraryLog.warning(
                    "search failed in \(entry.name, privacy: .public): \(error.localizedDescription, privacy: .public)"
                )
            }
        }
        guard !Task.isCancelled else { return }

        // A dictionary closed while the query ran must not come back as a group.
        searchGroups = groups.filter { handles[$0.dictionaryID] != nil }
        if let selectedKey, !hits.contains(where: { $0.key == selectedKey }) {
            self.selectedKey = nil
            entry = nil
        }
    }

    /// Read a resource (stylesheet, image…) from one open dictionary.
    ///
    /// Each section on the detail page resolves its own resources; a page-level
    /// lookup would serve the active dictionary's copy to every section.
    public func resource(for key: String, in dictionaryID: UUID) -> Data? {
        guard let handle = handles[dictionaryID] else { return nil }
        return try? handle.resource(for: key)
    }

    /// Resolve a key against one open dictionary without touching the selection.
    ///
    /// The detail page's sections navigate on their own: a link followed inside
    /// one dictionary must not move the others. `nil` means that dictionary has
    /// nothing for the key.
    public func entrySection(for key: String, in dictionaryID: UUID) -> DictionaryEntrySection? {
        guard let dictionary = openDictionaries.first(where: { $0.id == dictionaryID }),
              let resolved = resolveSection(key: key, dictionaryID: dictionaryID)
        else { return nil }
        return DictionaryEntrySection(
            dictionaryID: dictionaryID,
            name: displayName(of: dictionary),
            entry: resolved.entry,
            notice: resolved.notice
        )
    }

    /// Rebuild `entrySections` for the key on screen.
    ///
    /// The active dictionary leads and the rest follow the order they were
    /// opened in; a dictionary that does not carry the key adds no section.
    public func refreshEntrySections() {
        guard let key = selectedKey else {
            entrySections = []
            return
        }
        var sections: [DictionaryEntrySection] = []
        for dictionary in orderedOpenDictionaries() {
            // Every dictionary goes through the working-set-aware lookup,
            // including the active one: after a save `entry` still holds the
            // committed copy, so reusing it would keep showing stale text.
            let resolved: (entry: DictionaryEntry, notice: String?)? =
                resolveSection(key: key, dictionaryID: dictionary.id)
            guard let resolved else { continue }
            sections.append(
                DictionaryEntrySection(
                    dictionaryID: dictionary.id,
                    name: displayName(of: dictionary),
                    entry: resolved.entry,
                    notice: resolved.notice
                )
            )
        }
        entrySections = sections
    }

    /// Open dictionaries with the active one first, so it stays on top.
    private func orderedOpenDictionaries() -> [OpenDictionary] {
        guard let activeDictionaryID,
              let active = openDictionaries.first(where: { $0.id == activeDictionaryID })
        else { return openDictionaries }
        return [active] + openDictionaries.filter { $0.id != activeDictionaryID }
    }

    /// Name shown to readers; the library copy wins over the opened one.
    private func displayName(of dictionary: OpenDictionary) -> String {
        library.items.first { $0.id == dictionary.id }?.displayName ?? dictionary.name
    }

    /// One dictionary's answer for a key, or `nil` when it has none.
    private func resolveSection(
        key: String,
        dictionaryID: UUID
    ) -> (entry: DictionaryEntry, notice: String?)? {
        guard let handle = handles[dictionaryID],
              let result = try? handle.resolvePreferWorking(key)
        else { return nil }
        switch result {
        case let .entry(entry, requestedKey, _):
            let notice = entry.key == requestedKey ? nil : "重定向到 \(entry.key)"
            return (entry, notice)
        case let .deleted(requested):
            // Keep the section renderable: the notice carries the message and
            // the empty entry only supplies the header.
            return (
                DictionaryEntry(key: requested, display: requested),
                Self.workingDeletedNotice
            )
        default:
            return nil
        }
    }

    /// Load `key` from the active dictionary and show it.
    public func select(key: String) async {
        guard let dictionary = activeHandle else { return }
        let previous = entry
        selectedKey = key
        entry = nil
        errorMessage = nil
        noticeMessage = nil
        do {
            switch try dictionary.resolvePreferWorking(key) {
            case let .entry(loaded, requestedKey, _):
                guard selectedKey == key else { return }
                entry = loaded
                recordSearchHistory()
                if loaded.key != requestedKey {
                    await followRedirect(to: loaded)
                }
                if !isRestoringSession {
                    recordHistory(for: loaded)
                }
                scheduleSessionSave()
                selectionRevision += 1
            case let .missingTarget(target, _):
                entry = previous
                noticeMessage = "重定向目标不存在：\(target)"
            case let .cycle(chain):
                entry = previous
                noticeMessage = "重定向循环：\(chain.joined(separator: " → "))"
            case let .tooLong(chain):
                entry = previous
                noticeMessage = "重定向层数过多：\(chain.joined(separator: " → "))"
            case .deleted:
                guard selectedKey == key else { return }
                // Never fall back to the committed body: the reader must see
                // that the working set removed the word.
                entry = nil
                noticeMessage = Self.workingDeletedNotice
            case let .notFound(requested):
                entry = previous
                noticeMessage = "词典中没有这个词：\(requested)"
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        refreshEntrySections()
    }

    /// Show an entry a grouped result names, switching dictionaries first.
    ///
    /// History and bookmarks follow the target dictionary, not whichever one
    /// happened to be active before the tap.
    public func select(key: String, in dictionaryID: UUID) async {
        guard handles[dictionaryID] != nil else {
            noticeMessage = "这本词典已关闭。"
            return
        }
        if activeDictionaryID != dictionaryID {
            activateDictionary(id: dictionaryID)
        }
        await select(key: key)
    }

    /// A redirect resolves to a different key: show it and its neighbourhood.
    private func followRedirect(to loaded: DictionaryEntry) async {
        selectedKey = loaded.key
        query = loaded.display
        guard let dictionary = activeHandle, let dictionaryID = activeDictionaryID else { return }
        let results = (try? dictionary.search(loaded.key)) ?? []
        let hits = results.contains(where: { $0.key == loaded.key })
            ? results
            : [SearchHit(key: loaded.key, display: loaded.display)] + results
        updateHits(hits, in: dictionaryID)
    }

    /// Replace one dictionary's hits without disturbing the other groups.
    private func updateHits(_ hits: [SearchHit], in dictionaryID: UUID) {
        if let index = searchGroups.firstIndex(where: { $0.dictionaryID == dictionaryID }) {
            searchGroups[index] = DictionarySearchGroup(
                dictionaryID: dictionaryID,
                name: searchGroups[index].name,
                hits: hits
            )
            return
        }
        guard let order = openDictionaries.firstIndex(where: { $0.id == dictionaryID })
        else { return }
        let group = DictionarySearchGroup(
            dictionaryID: dictionaryID,
            name: openDictionaries[order].name,
            hits: hits
        )
        let insertAt = searchGroups.firstIndex { existing in
            guard let index = openDictionaries.firstIndex(where: { $0.id == existing.dictionaryID }) else {
                return false
            }
            return index > order
        } ?? searchGroups.count
        searchGroups.insert(group, at: insertAt)
    }

    /// Record the current (trimmed) query in the search history, if any.
    public func recordSearchHistory() {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        searchHistory.record(trimmed)
    }

    /// Open the first hit of the active dictionary — what Return does.
    public func submitQuery() {
        recordSearchHistory()
        guard let first = hits.first else { return }
        Task { await select(key: first.key) }
    }

    // MARK: - Errors

    public func present(error: any Error) {
        errorMessage = error.localizedDescription
    }

    public func dismissError() {
        errorMessage = nil
    }
}

extension DictionaryStore: SessionLookup {
    func sessionHasDictionary(id: UUID) -> Bool {
        library.item(id: id) != nil && handles[id] != nil
    }

    func sessionHasEntry(key: String, in dictionaryID: UUID) -> Bool {
        guard let handle = handles[dictionaryID],
              let result = try? handle.resolve(key)
        else { return false }
        if case .entry = result { return true }
        return false
    }
}
