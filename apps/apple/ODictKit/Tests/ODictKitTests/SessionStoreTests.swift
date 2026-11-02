import XCTest
@testable import ODictKit

final class SessionStoreTests: XCTestCase {
    private func makeDirectory() throws -> URL {
        let directory = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testRoundTripTrimsAndKeepsEveryField() throws {
        let directory = try makeDirectory()
        let id = UUID().uuidString
        let session = ReaderSession(
            dictionaryID: "  \(id)  ",
            key: " apple ",
            query: " apple\n"
        )

        try SessionStore.save(session, in: directory)
        let loaded = SessionStore.load(in: directory)

        XCTAssertEqual(
            loaded,
            ReaderSession(dictionaryID: id, key: "apple", query: "apple")
        )
    }

    func testBlankFieldsAreNotWritten() throws {
        let directory = try makeDirectory()
        let id = UUID().uuidString

        try SessionStore.save(
            ReaderSession(dictionaryID: id, key: "   ", query: ""),
            in: directory
        )

        XCTAssertEqual(
            SessionStore.load(in: directory),
            ReaderSession(dictionaryID: id, key: nil, query: nil)
        )
    }

    func testEmptySessionRemovesTheFile() throws {
        let directory = try makeDirectory()
        try SessionStore.save(
            ReaderSession(dictionaryID: UUID().uuidString, key: "apple", query: nil),
            in: directory
        )
        XCTAssertTrue(FileManager.default.fileExists(atPath: SessionStore.fileURL(in: directory).path))

        try SessionStore.save(.empty, in: directory)

        XCTAssertFalse(FileManager.default.fileExists(atPath: SessionStore.fileURL(in: directory).path))
        XCTAssertEqual(SessionStore.load(in: directory), .empty)
    }

    func testMissingFileLoadsEmptySession() throws {
        let directory = try makeDirectory()
        let result = SessionStore.loadResult(in: directory)

        XCTAssertEqual(result.session, .empty)
        XCTAssertFalse(result.recoveredFromCorruptFile)
    }

    func testCorruptFileIsDiscardedQuietly() throws {
        let directory = try makeDirectory()
        try Data("not json".utf8).write(to: SessionStore.fileURL(in: directory), options: [.atomic])

        let result = SessionStore.loadResult(in: directory)

        XCTAssertEqual(result.session, .empty)
        XCTAssertTrue(result.recoveredFromCorruptFile)
    }
}

@MainActor
final class SessionRestoreTests: XCTestCase {
    @MainActor
    private final class StubLookup: SessionLookup {
        var dictionaryID: UUID?
        var entries: [UUID: Set<String>] = [:]

        func sessionHasDictionary(id: UUID) -> Bool {
            dictionaryID == id
        }

        func sessionHasEntry(key: String, in dictionaryID: UUID) -> Bool {
            entries[dictionaryID]?.contains(key) ?? false
        }
    }

    func testMissingDictionaryDropsTheWholePlan() {
        let lookup = StubLookup()
        let session = ReaderSession(
            dictionaryID: UUID().uuidString,
            key: "apple",
            query: "app"
        )

        XCTAssertNil(SessionRestore.plan(for: session, using: lookup))
    }

    func testMissingEntryKeepsTheQuery() {
        let id = UUID()
        let lookup = StubLookup()
        lookup.dictionaryID = id
        let session = ReaderSession(dictionaryID: id.uuidString, key: "gone", query: "app")

        let plan = SessionRestore.plan(for: session, using: lookup)

        XCTAssertEqual(plan, SessionRestorePlan(dictionaryID: id, key: nil, query: "app"))
    }

    func testExistingEntryIsRestored() {
        let id = UUID()
        let lookup = StubLookup()
        lookup.dictionaryID = id
        lookup.entries[id] = ["apple"]
        let session = ReaderSession(dictionaryID: id.uuidString, key: "apple", query: nil)

        let plan = SessionRestore.plan(for: session, using: lookup)

        XCTAssertEqual(plan, SessionRestorePlan(dictionaryID: id, key: "apple", query: nil))
    }

    func testInvalidIdentifierDropsThePlan() {
        let lookup = StubLookup()
        let session = ReaderSession(dictionaryID: "not-a-uuid", key: "apple", query: nil)

        XCTAssertNil(SessionRestore.plan(for: session, using: lookup))
    }
}
