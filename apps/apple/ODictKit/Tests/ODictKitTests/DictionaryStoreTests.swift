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
