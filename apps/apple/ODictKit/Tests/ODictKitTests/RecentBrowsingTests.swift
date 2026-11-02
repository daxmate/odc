import XCTest
@testable import ODictKit

final class RecentBrowsingTests: XCTestCase {
    private let dictionary = UUID(uuidString: "33333333-3333-3333-3333-333333333333")!
    private let time = Date(timeIntervalSince1970: 1_760_000_000)

    private func ref(_ key: String, seconds: TimeInterval) -> EntryRef {
        EntryRef(
            dictionaryID: dictionary.uuidString,
            key: key,
            headword: key.uppercased(),
            timestamp: time.addingTimeInterval(seconds)
        )
    }

    func testRowsKeepHistoryOrderAndCarryTheDictionaryName() {
        let rows = RecentBrowsing.rows(
            from: [ref("three", seconds: 30), ref("two", seconds: 20), ref("one", seconds: 10)],
            dictionaryNames: [dictionary: "Demo"]
        )

        XCTAssertEqual(rows.map(\.ref.key), ["three", "two", "one"])
        XCTAssertEqual(rows.first?.dictionaryName, "Demo")
    }

    func testRowsAreCappedAtEight() {
        let history = (0..<12).map { index in
            ref("word\(index)", seconds: TimeInterval(index))
        }

        let rows = RecentBrowsing.rows(from: history, dictionaryNames: [dictionary: "Demo"])

        XCTAssertEqual(rows.count, RecentBrowsing.limit)
        XCTAssertEqual(rows.first?.ref.key, "word0")
        XCTAssertEqual(rows.last?.ref.key, "word7")
    }

    func testRowsDropDuplicatesAndKeepTheFirstOccurrence() {
        let newer = ref("apple", seconds: 30)
        let older = ref("apple", seconds: 10)

        let rows = RecentBrowsing.rows(
            from: [newer, older],
            dictionaryNames: [dictionary: "Demo"]
        )

        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows.first?.ref.timestamp, newer.timestamp)
    }

    func testUnknownDictionaryUsesAPlaceholderName() {
        let rows = RecentBrowsing.rows(
            from: [ref("apple", seconds: 10)],
            dictionaryNames: [:]
        )

        XCTAssertEqual(rows.first?.dictionaryName, "已移除的词典")
    }

    func testEmptyHistoryProducesNoSection() {
        XCTAssertTrue(RecentBrowsing.rows(from: [], dictionaryNames: [:]).isEmpty)
        XCTAssertTrue(
            RecentBrowsing.rows(from: [ref("apple", seconds: 10)], dictionaryNames: [:], limit: 0).isEmpty
        )
    }
}
