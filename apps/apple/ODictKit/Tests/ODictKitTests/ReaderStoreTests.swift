import XCTest
@testable import ODictKit

final class ReaderStoreTests: XCTestCase {
    private func makeDirectory() throws -> URL {
        let directory = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    private func reference(
        _ key: String,
        dictionary: String = "dictionary",
        at seconds: TimeInterval = 1_760_000_000
    ) -> EntryRef {
        EntryRef(
            dictionaryID: dictionary,
            key: key,
            headword: key.capitalized,
            timestamp: Date(timeIntervalSince1970: seconds)
        )
    }

    func testSaveAndLoadRoundTrip() throws {
        let directory = try makeDirectory()
        let ref = reference("apple")
        var state = ReaderState()
        ReaderStore.recordHistory(ref, in: &state)
        ReaderStore.toggleBookmark(ref, in: &state)

        try ReaderStore.save(state, in: directory)

        XCTAssertEqual(ReaderStore.load(in: directory), state)
        XCTAssertEqual(ReaderStore.fileURL(in: directory).lastPathComponent, "reader.json")
    }

    func testMissingFileLoadsAnEmptyStateWithoutARecoveryNotice() throws {
        let directory = try makeDirectory()

        let result = ReaderStore.loadResult(in: directory)

        XCTAssertEqual(result.state, ReaderState())
        XCTAssertFalse(result.recoveredFromCorruptFile)
    }

    func testCorruptFileFallsBackToAnEmptyState() throws {
        let directory = try makeDirectory()
        try Data("{ not json".utf8).write(to: ReaderStore.fileURL(in: directory))

        let result = ReaderStore.loadResult(in: directory)

        XCTAssertEqual(result.state, ReaderState())
        XCTAssertTrue(result.recoveredFromCorruptFile)
    }

    func testHistoryKeepsOneRecordPerEntryWithTheNewestFirst() {
        var state = ReaderState()
        ReaderStore.recordHistory(reference("apple", at: 10), in: &state)
        ReaderStore.recordHistory(reference("application", at: 20), in: &state)
        ReaderStore.recordHistory(reference("apple", at: 30), in: &state)

        XCTAssertEqual(state.history.map(\.key), ["apple", "application"])
        XCTAssertEqual(state.history.first?.timestamp, Date(timeIntervalSince1970: 30))
    }

    func testHistoryIsSharedPerDictionaryNotPerKey() {
        var state = ReaderState()
        ReaderStore.recordHistory(reference("apple", dictionary: "first"), in: &state)
        ReaderStore.recordHistory(reference("apple", dictionary: "second"), in: &state)

        XCTAssertEqual(state.history.map(\.dictionaryID), ["second", "first"])
    }

    func testHistoryDropsTheOldestEntryBeyondTheLimit() {
        var state = ReaderState()
        for index in 0..<(ReaderStore.historyLimit + 5) {
            ReaderStore.recordHistory(reference("word-\(index)", at: Double(index)), in: &state)
        }

        XCTAssertEqual(state.history.count, ReaderStore.historyLimit)
        XCTAssertEqual(state.history.first?.key, "word-\(ReaderStore.historyLimit + 4)")
        XCTAssertEqual(state.history.last?.key, "word-5")
    }

    func testBookmarksAreSortedNewestFirst() {
        var state = ReaderState()
        ReaderStore.toggleBookmark(reference("old", at: 10), in: &state)
        ReaderStore.toggleBookmark(reference("new", at: 20), in: &state)
        ReaderStore.toggleBookmark(reference("middle", at: 15), in: &state)

        XCTAssertEqual(state.bookmarks.map(\.key), ["new", "middle", "old"])
    }

    func testTogglingTheSameEntryTwiceLeavesNoBookmark() {
        var state = ReaderState()
        ReaderStore.toggleBookmark(reference("apple"), in: &state)
        ReaderStore.toggleBookmark(reference("apple", at: 99), in: &state)

        XCTAssertTrue(state.bookmarks.isEmpty)
    }

    func testForgettingADictionaryDropsItsReferences() {
        var state = ReaderState()
        ReaderStore.recordHistory(reference("apple", dictionary: "gone"), in: &state)
        ReaderStore.recordHistory(reference("apple", dictionary: "kept"), in: &state)
        ReaderStore.toggleBookmark(reference("apple", dictionary: "gone"), in: &state)

        ReaderStore.forget(dictionaryID: "gone", in: &state)

        XCTAssertEqual(state.history.map(\.dictionaryID), ["kept"])
        XCTAssertTrue(state.bookmarks.isEmpty)
    }

    func testEntryIDCombinesTheDictionaryAndTheKey() {
        XCTAssertEqual(reference("apple", dictionary: "A").id, "A#apple")
    }

    func testNormalisationRepairsADuplicatedHandEditedFile() throws {
        let directory = try makeDirectory()
        let older = reference("apple", at: 10)
        let newer = reference("apple", at: 20)
        let state = ReaderState(bookmarks: [older, newer], history: [older, older])
        try ReaderStore.save(state, in: directory)

        let loaded = ReaderStore.load(in: directory)

        XCTAssertEqual(loaded.bookmarks, [newer])
        XCTAssertEqual(loaded.history, [older])
    }
}
