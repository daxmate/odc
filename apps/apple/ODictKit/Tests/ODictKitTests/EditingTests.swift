import XCTest
@testable import ODictKit

@MainActor
final class EditingTests: XCTestCase {
    /// Every test edits a private copy so the version-controlled fixture stays
    /// untouched.
    private func openCopy() throws -> OdcDictionary {
        let container = try Fixture.temporaryDirectory()
        addTeardownBlock {
            try? FileManager.default.removeItem(at: container)
        }
        let copy = try DocumentLibrary.copyDocument(at: Fixture.demoRepository, into: container)
        return try OdcDictionary(url: copy)
    }

    func testEntriesPageWalksKeysInOrder() throws {
        let dictionary = try openCopy()
        let first = try dictionary.entriesPage(limit: 2)
        XCTAssertEqual(first.map(\.key), ["apple", "application"])
        let rest = try dictionary.entriesPage(after: first.last?.key, limit: 2)
        XCTAssertEqual(rest.map(\.key), ["banana"])
        XCTAssertTrue(try dictionary.entriesPage(after: "banana", limit: 2).isEmpty)
    }

    func testEditingRoundTrip() throws {
        let dictionary = try openCopy()
        XCTAssertTrue(try dictionary.workingStatus().isClean)

        let added = try dictionary.upsert(
            DictionaryEntry(
                key: "cherry",
                display: "Cherry",
                defs: [Definition(pos: "noun", sense: "a small fruit")]
            )
        )
        XCTAssertEqual(added, EntryWriteResult(key: "cherry", outcome: .added))

        let apple = try dictionary.entry(for: "apple")
        let edited = DictionaryEntry(
            key: apple.key,
            display: "Apple",
            phonetic: apple.phonetic,
            defs: apple.defs,
            rawHTML: "<p>apple</p>"
        )
        XCTAssertEqual(try dictionary.upsert(edited).outcome, .modified)

        XCTAssertEqual(try dictionary.deleteEntry("banana").outcome, .deleted)
        XCTAssertEqual(try dictionary.deleteEntry("missing").outcome, .notFound)

        let status = try dictionary.workingStatus()
        XCTAssertEqual(status.added, 1)
        XCTAssertEqual(status.modified, 1)
        XCTAssertEqual(status.removed, 1)
        XCTAssertEqual(status.total, 3)

        let changes = try dictionary.workingChanges()
        XCTAssertEqual(changes.map(\.key), ["apple", "banana", "cherry"])
        XCTAssertEqual(changes.map(\.kind), [.modified, .deleted, .added])

        let diff = try dictionary.entryDiff("cherry")
        XCTAssertEqual(diff.kind, .added)
        XCTAssertTrue(diff.fields.allSatisfy { $0.before == nil })
        XCTAssertTrue(diff.changedFields.contains { $0.field == "defs[0].sense" })

        let commit = try dictionary.commit(message: "edit", author: "tester")
        XCTAssertFalse(commit.hash.isEmpty)
        XCTAssertEqual(commit.entryCount, 3)
        XCTAssertEqual(commit.entries, 3)
        XCTAssertTrue(try dictionary.workingStatus().isClean)

        _ = try dictionary.upsert(DictionaryEntry(key: "date", display: "Date"))
        XCTAssertTrue(try dictionary.discardWorking().discarded)
        XCTAssertTrue(try dictionary.workingStatus().isClean)
        XCTAssertEqual(
            try dictionary.entriesPage().map(\.key),
            ["apple", "application", "cherry"]
        )
    }

    func testWorkingResolutionPrefersEditsAndReportsDeletions() throws {
        let dictionary = try openCopy()
        defer { dictionary.close() }

        _ = try dictionary.upsert(
            DictionaryEntry(key: "apple", display: "Apple (saved)")
        )
        guard case let .entry(edited, _, hops) = try dictionary.resolvePreferWorking("apple")
        else {
            return XCTFail("expected the working copy")
        }
        XCTAssertEqual(edited.display, "Apple (saved)")
        XCTAssertEqual(hops, 0)

        // The committed lookup keeps reading HEAD.
        guard case let .entry(committed, _, _) = try dictionary.resolve("apple") else {
            return XCTFail("expected the committed entry")
        }
        XCTAssertEqual(committed.display, "apple")

        _ = try dictionary.deleteEntry("banana")
        XCTAssertEqual(try dictionary.resolvePreferWorking("banana"), .deleted("banana"))
        guard case .entry = try dictionary.resolve("banana") else {
            return XCTFail("the committed lookup must still find banana")
        }
    }
}
