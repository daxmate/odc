import XCTest
@testable import ODictKit

@MainActor
final class DictionaryStoreTests: XCTestCase {
    func testBlankQueryProducesNoHits() {
        let store = DictionaryStore()
        store.setQuery("   ")
        XCTAssertTrue(store.hits.isEmpty)
        XCTAssertEqual(store.query, "   ")
    }

    func testMDXFilesRouteToTheImporter() {
        let store = DictionaryStore()
        defer { store.close() }

        store.openDocument(url: URL(fileURLWithPath: "/tmp/missing.mdx"))

        XCTAssertEqual(store.importPhase, .preparing)
        XCTAssertNil(store.documentURL)
    }

    func testODCFilesStillOpenDirectly() {
        let store = DictionaryStore()
        defer { store.close() }

        store.openDocument(url: Fixture.demoRepository)

        XCTAssertNil(store.importPhase)
    }

    func testQueryWithoutADocumentIsIgnored() {
        let store = DictionaryStore()
        store.setQuery("apple")
        XCTAssertTrue(store.hits.isEmpty)
        XCTAssertNil(store.selectedKey)
    }

    func testOpeningAFixtureCopyPopulatesState() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let copy = try DocumentLibrary.copyDocument(at: Fixture.demoRepository, into: container)
        let dictionary = try OdcDictionary(url: copy)
        XCTAssertEqual(try dictionary.search("app", limit: 50).count, 2)
    }

    func testOpeningAURLPopulatesTheStore() async throws {
        let store = DictionaryStore(container: try makeContainer())
        defer { store.close() }

        // Same entry point the document handlers use (onOpenURL / NSApplicationDelegate).
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }

        XCTAssertEqual(store.documentName, "demo")
        XCTAssertNotNil(store.documentURL)
        XCTAssertNil(store.errorMessage)

        store.setQuery("app")
        try await waitUntil { !store.hits.isEmpty }
        XCTAssertEqual(store.hits.map(\.key), ["apple", "application"])
    }

    func testSelectingAnEntryRecordsTheQuery() async throws {
        let suiteName = "DictionaryStoreTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = DictionaryStore(
            container: try makeContainer(),
            searchHistory: SearchHistoryStore(userDefaults: defaults, storageKey: "history")
        )
        defer { store.close() }

        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }

        store.setQuery("app")
        try await waitUntil { !store.hits.isEmpty }
        await store.select(key: "apple")

        XCTAssertNotNil(store.entry)
        XCTAssertEqual(store.searchHistory.items, ["app"])
    }

    func testSubmittingAQueryRecordsItEvenWithoutHits() {
        let suiteName = "DictionaryStoreTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = DictionaryStore(
            searchHistory: SearchHistoryStore(userDefaults: defaults, storageKey: "history")
        )
        store.setQuery("  apple ")
        store.submitQuery()

        XCTAssertEqual(store.searchHistory.items, ["apple"])
    }

    func testCloseAfterOpeningRemovesTheCopy() async throws {
        let store = DictionaryStore(container: try makeContainer())
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }
        let copy = try XCTUnwrap(store.documentURL)

        store.close()
        XCTAssertNil(store.documentURL)
        XCTAssertFalse(FileManager.default.fileExists(atPath: copy.path))
    }

    /// A throwaway library root, so tests never read or write the user's real
    /// Application Support/ODict folder.
    // MARK: - Startup migration

    func testRestoreMovesAnExternalDictionaryIntoTheLibrary() async throws {
        let container = try makeContainer()
        let outside = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: outside) }
        let externalCopy = outside.appending(path: "demo.odc", directoryHint: .isDirectory)
        try FileManager.default.copyItem(at: Fixture.demoRepository, to: externalCopy)
        let item = LibraryItem(
            displayName: "demo",
            storedName: externalCopy.path,
            sourceName: "demo",
            sourceBytes: 4096
        )
        try LibraryStore.save(
            LibraryState(items: [item], lastOpenedID: item.id),
            to: container
        )

        let store = DictionaryStore(container: container)
        defer { store.close() }
        store.restore()
        try await waitUntil { store.documentURL != nil }

        XCTAssertNil(store.errorMessage)
        XCTAssertTrue(store.migrationIssues.isEmpty)
        let opened = try XCTUnwrap(store.documentURL)
        XCTAssertEqual(opened.path, container.appending(path: "\(item.id.uuidString).odc").path)
        // The index now points into the library, so the next launch skips the move.
        let reloaded = LibraryStore.load(from: container)
        XCTAssertEqual(reloaded.items.count, 1)
        XCTAssertTrue(LibraryStore.isStored(reloaded.items[0], in: container))
        XCTAssertTrue(FileManager.default.fileExists(atPath: externalCopy.appending(path: "odc.db").path))
    }

    func testRestoreKeepsAnUnreadableEntryAndReportsIt() async throws {
        let container = try makeContainer()
        let outside = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: outside) }
        let broken = outside.appending(path: "broken.odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: broken, withIntermediateDirectories: true)
        try Data("not a database".utf8).write(to: broken.appending(path: "odc.db"))
        let item = LibraryItem(
            displayName: "broken",
            storedName: broken.path,
            sourceName: "broken",
            sourceBytes: 64
        )
        try LibraryStore.save(LibraryState(items: [item]), to: container)

        let store = DictionaryStore(container: container)
        defer { store.close() }
        store.restore()
        try await waitUntil { !store.migrationIssues.isEmpty }

        XCTAssertNil(store.documentURL)
        XCTAssertEqual(store.migrationIssues.count, 1)
        XCTAssertEqual(store.migrationIssues[0].displayName, "broken")
        XCTAssertEqual(LibraryStore.load(from: container).items.count, 1)
    }

    private func makeContainer() throws -> URL {
        let container = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: container) }
        return container
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        _ condition: @MainActor () -> Bool
    ) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            guard ContinuousClock.now < deadline else {
                XCTFail("condition not met within \(timeout)")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    func testDismissErrorClearsMessage() {
        let store = DictionaryStore()
        store.present(error: OdcError.closed)
        XCTAssertNotNil(store.errorMessage)
        store.dismissError()
        XCTAssertNil(store.errorMessage)
    }


    func testSubmitQueryOpensTheFirstHit() async throws {
        let store = DictionaryStore(container: try makeContainer())
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }
        defer { store.close() }

        store.setQuery("app")
        try await waitUntil { !store.hits.isEmpty }
        XCTAssertNil(store.entry)

        store.submitQuery()
        try await waitUntil { store.entry?.key == "apple" }
        XCTAssertEqual(store.selectedKey, "apple")
        XCTAssertEqual(store.query, "app")
    }

    func testSubmitQueryWithoutHitsIsANoOp() async throws {
        let store = DictionaryStore(container: try makeContainer())
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }
        defer { store.close() }

        store.setQuery("zzzzzz")
        try await Task.sleep(for: .milliseconds(400))
        XCTAssertTrue(store.hits.isEmpty)

        store.submitQuery()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNil(store.selectedKey)
        XCTAssertEqual(store.query, "zzzzzz")
    }

    func testImportSummaryClearsItself() async throws {
        let store = DictionaryStore(importSummaryLifetime: .milliseconds(60))
        store.presentImportSummary("导入完成：1 本")
        XCTAssertEqual(store.importSummary, "导入完成：1 本", "the note is visible first")

        try await waitUntil { store.importSummary == nil }
    }

    func testImportSummaryDismissesOnDemand() async throws {
        let store = DictionaryStore(importSummaryLifetime: .seconds(60))
        store.presentImportSummary("导入完成：1 本")
        store.dismissImportSummary()
        XCTAssertNil(store.importSummary)

        // The cancelled timer must not resurrect or clear anything else.
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertNil(store.importSummary)
    }

    func testCloseResetsState() {
        let store = DictionaryStore()
        store.close()
        XCTAssertNil(store.documentURL)
        XCTAssertNil(store.documentName)
        XCTAssertNil(store.errorMessage)
        XCTAssertTrue(store.hits.isEmpty)
        XCTAssertFalse(store.isLoading)
    }
}
