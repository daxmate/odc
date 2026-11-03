import Foundation
import XCTest
@testable import ODictKit

@MainActor
final class PublishHistoryTests: XCTestCase {
    func testFreshStoreHasNoRecord() {
        let store = PublishHistoryStore(defaults: PublishingFixtures.defaults())

        XCTAssertNil(store.record(for: "library-a"))
    }

    func testUpdateIsReadBackForTheSameLibrary() {
        let store = PublishHistoryStore(defaults: PublishingFixtures.defaults())
        let record = PublishRecord(head: "abcdef0123456789", label: "2026.11.03", publishedAt: Date(timeIntervalSince1970: 1_700_000_000))

        store.update(record, for: "library-a")

        XCTAssertEqual(store.record(for: "library-a"), record)
        XCTAssertEqual(store.record(for: "library-a")?.shortHead, "abcdef0")
        XCTAssertTrue(store.key(for: "library-a").hasPrefix(PublishHistoryStore.keyPrefix))
        XCTAssertTrue(store.key(for: "library-a").hasSuffix("library-a"))
    }

    func testLibrariesDoNotShareRecords() {
        let store = PublishHistoryStore(defaults: PublishingFixtures.defaults())
        store.update(PublishRecord(head: "aaa", label: "1"), for: "library-a")
        store.update(PublishRecord(head: "bbb", label: "2"), for: "library-b")

        XCTAssertEqual(store.record(for: "library-a")?.head, "aaa")
        XCTAssertEqual(store.record(for: "library-b")?.head, "bbb")
    }

    func testClearingRemovesOnlyThatLibrary() {
        let store = PublishHistoryStore(defaults: PublishingFixtures.defaults())
        store.update(PublishRecord(head: "aaa", label: "1"), for: "library-a")
        store.update(PublishRecord(head: "bbb", label: "2"), for: "library-b")

        store.clear(for: "library-a")

        XCTAssertNil(store.record(for: "library-a"))
        XCTAssertEqual(store.record(for: "library-b")?.head, "bbb")
    }

    func testModelUsesTheLastHeadAsPatchBase() {
        let history = PublishHistoryStore(defaults: PublishingFixtures.defaults())
        history.update(PublishRecord(head: "abcdef0123456789", label: "2026.10.01"), for: "library-a")
        let model = PublishModel(
            repoPath: "/tmp/odc-publishing-tests",
            libraryKey: "library-a",
            libraryName: "demo",
            keyStore: InMemoryPublisherKeyStore(),
            history: history
        )

        XCTAssertTrue(model.canIncludePatch)
        XCTAssertNil(model.patchBase)

        model.includePatch = true

        XCTAssertEqual(model.patchBase, "abcdef0123456789")
    }

    func testModelWithoutHistoryCannotIncludePatch() {
        let model = PublishModel(
            repoPath: "/tmp/odc-publishing-tests",
            libraryKey: "library-b",
            libraryName: "demo",
            keyStore: InMemoryPublisherKeyStore(),
            history: PublishHistoryStore(defaults: PublishingFixtures.defaults())
        )

        XCTAssertFalse(model.canIncludePatch)
        model.includePatch = true
        XCTAssertNil(model.patchBase)
    }

    func testDefaultLabelIsDottedDate() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .current
        let now = calendar.date(from: DateComponents(year: 2026, month: 3, day: 9, hour: 12)) ?? Date()

        XCTAssertEqual(PublishModel.defaultLabel(now: now, calendar: calendar), "2026.03.09")
    }
}
