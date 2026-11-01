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
