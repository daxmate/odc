import XCTest
@testable import ODictKit

@MainActor
final class ReaderMemoryTests: XCTestCase {
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

    /// Opens the fixture dictionary and waits for it to be on screen.
    private func openDemo(in container: URL) async throws -> DictionaryStore {
        let store = DictionaryStore(container: container)
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil || store.errorMessage != nil }
        XCTAssertNil(store.errorMessage)
        XCTAssertNotNil(store.documentURL)
        return store
    }

    func testSelectingAnEntryRecordsIt() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)
        defer { store.close() }

        await store.select(key: "apple")

        XCTAssertEqual(store.entry?.key, "apple")
        XCTAssertEqual(store.recentEntries.map(\.key), ["apple"])
        XCTAssertEqual(store.recentEntries.first?.headword, store.entry?.display)
        XCTAssertEqual(ReaderStore.load(in: container).history.map(\.key), ["apple"])
    }

    func testAWholeQueryRecordsOnlyTheEntryThatResolved() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)
        defer { store.close() }

        await store.select(key: "apple")
        await store.select(key: "application")
        await store.select(key: "apple")

        XCTAssertEqual(store.recentEntries.map(\.key), ["apple", "application"])
    }

    func testAMissingEntryIsNotRecorded() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)
        defer { store.close() }

        await store.select(key: "no-such-word")

        XCTAssertNil(store.entry)
        XCTAssertTrue(store.recentEntries.isEmpty)
        XCTAssertTrue(ReaderStore.load(in: container).history.isEmpty)
    }

    func testTogglingTheBookmarkOfTheEntryOnScreen() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)
        defer { store.close() }

        await store.select(key: "apple")
        XCTAssertFalse(store.isBookmarked(key: "apple"))

        store.toggleBookmark()

        XCTAssertTrue(store.isBookmarked(key: "apple"))
        XCTAssertEqual(store.bookmarkedEntries.map(\.key), ["apple"])
        XCTAssertEqual(ReaderStore.load(in: container).bookmarks.map(\.key), ["apple"])

        store.toggleBookmark()

        XCTAssertFalse(store.isBookmarked(key: "apple"))
        XCTAssertTrue(ReaderStore.load(in: container).bookmarks.isEmpty)
    }

    func testBookmarkingWithoutAnEntryDoesNothing() throws {
        let container = try makeContainer()
        let store = DictionaryStore(container: container)

        store.toggleBookmark()

        XCTAssertTrue(store.bookmarkedEntries.isEmpty)
    }

    func testRemovingADictionaryDropsItsBookmarksAndHistory() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)
        let item = try XCTUnwrap(store.library.items.first)

        await store.select(key: "apple")
        store.toggleBookmark()

        store.removeItem(id: item.id)

        XCTAssertTrue(store.recentEntries.isEmpty)
        XCTAssertTrue(store.bookmarkedEntries.isEmpty)
        let saved = ReaderStore.load(in: container)
        XCTAssertTrue(saved.history.isEmpty)
        XCTAssertTrue(saved.bookmarks.isEmpty)
    }

    func testClosingADictionaryDropsItsBookmarksAndHistory() async throws {
        let container = try makeContainer()
        let store = try await openDemo(in: container)

        await store.select(key: "apple")
        store.toggleBookmark()

        store.close()

        XCTAssertTrue(store.recentEntries.isEmpty)
        let saved = ReaderStore.load(in: container)
        XCTAssertTrue(saved.history.isEmpty)
        XCTAssertTrue(saved.bookmarks.isEmpty)
    }

    func testACorruptReaderFileIsReportedAndReset() async throws {
        let container = try makeContainer()
        try Data("not json".utf8).write(to: ReaderStore.fileURL(in: container))
        let store = DictionaryStore(container: container)

        store.restore()
        await store.awaitRestore()

        XCTAssertEqual(store.noticeMessage, "阅读记录文件已损坏，已重置。")
        XCTAssertTrue(store.recentEntries.isEmpty)
        XCTAssertEqual(ReaderStore.loadResult(in: container).recoveredFromCorruptFile, true)
    }
}
