import XCTest
@testable import ODictKit

final class WordbookPresentationTests: XCTestCase {
    private let firstDictionary = UUID(uuidString: "11111111-1111-1111-1111-111111111111")!
    private let secondDictionary = UUID(uuidString: "22222222-2222-2222-2222-222222222222")!
    private let time = Date(timeIntervalSince1970: 1_760_000_000)

    private var names: [UUID: String] {
        [firstDictionary: "A", secondDictionary: "B"]
    }

    private func ref(
        _ key: String,
        headword: String? = nil,
        dictionary: UUID? = nil,
        seconds: TimeInterval
    ) -> EntryRef {
        EntryRef(
            dictionaryID: (dictionary ?? firstDictionary).uuidString,
            key: key,
            headword: headword ?? key,
            timestamp: time.addingTimeInterval(seconds)
        )
    }

    func testAllReturnsTheUnionWithBookmarkFlag() {
        let bookmarked = ref("zebra", seconds: 10)
        let opened = ref("apple", seconds: 20)

        let presentation = WordbookPresentation(
            bookmarks: [bookmarked],
            history: [opened],
            filter: .all,
            dictionaryID: nil,
            sort: .recent,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["apple", "zebra"])
        XCTAssertEqual(presentation.entries.first?.isBookmarked, false)
        XCTAssertEqual(presentation.entries.last?.isBookmarked, true)
        XCTAssertEqual(presentation.count, 2)
    }

    func testBookmarkFilterKeepsOnlyBookmarks() {
        let bookmarked = ref("zebra", seconds: 10)
        let opened = ref("apple", seconds: 20)

        let presentation = WordbookPresentation(
            bookmarks: [bookmarked],
            history: [opened, bookmarked],
            filter: .bookmarks,
            dictionaryID: nil,
            sort: .recent,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["zebra"])
        XCTAssertTrue(presentation.entries.allSatisfy(\.isBookmarked))
    }

    func testHistoryFilterKeepsOnlyBrowsedEntries() {
        let bookmarked = ref("zebra", seconds: 10)
        let opened = ref("apple", seconds: 20)

        let presentation = WordbookPresentation(
            bookmarks: [bookmarked],
            history: [opened],
            filter: .history,
            dictionaryID: nil,
            sort: .recent,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["apple"])
        XCTAssertEqual(presentation.emptyMessage, "还没有浏览记录")
    }

    func testDictionaryScopeKeepsOnlyThatDictionary() {
        let fromFirst = ref("apple", seconds: 20)
        let fromSecond = ref("banana", dictionary: secondDictionary, seconds: 30)

        let presentation = WordbookPresentation(
            bookmarks: [],
            history: [fromSecond, fromFirst],
            filter: .all,
            dictionaryID: secondDictionary,
            sort: .recent,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["banana"])
        XCTAssertEqual(presentation.entries.first?.dictionaryName, "B")
    }

    func testAlphabeticalSortUsesLocalizedStandardCompare() {
        let zebra = ref("zebra", seconds: 30)
        let apple = ref("apple", seconds: 10)
        let banana = ref("banana", seconds: 20)

        let presentation = WordbookPresentation(
            bookmarks: [],
            history: [zebra, banana, apple],
            filter: .all,
            dictionaryID: nil,
            sort: .alphabetical,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["apple", "banana", "zebra"])
    }

    func testRecentSortIsNewestFirst() {
        let older = ref("zebra", seconds: 10)
        let newer = ref("apple", seconds: 30)

        let presentation = WordbookPresentation(
            bookmarks: [older],
            history: [older, newer],
            filter: .all,
            dictionaryID: nil,
            sort: .recent,
            dictionaryNames: names
        )

        XCTAssertEqual(presentation.entries.map(\.ref.key), ["apple", "zebra"])
    }

    func testEmptyStateMessageFollowsTheFilter() {
        let empty: ([EntryRef], WordbookFilter) -> String = { bookmarks, filter in
            WordbookPresentation(
                bookmarks: bookmarks,
                history: [],
                filter: filter,
                dictionaryID: nil,
                sort: .recent,
                dictionaryNames: self.names
            ).emptyMessage
        }

        XCTAssertEqual(empty([], .all), "生词本还是空的")
        XCTAssertEqual(empty([], .bookmarks), "还没有收藏")
        XCTAssertEqual(empty([], .history), "还没有浏览记录")
    }

    func testUnknownDictionaryFallsBackToAPlaceholderName() {
        let ref = ref("apple", seconds: 10)

        let presentation = WordbookPresentation(
            bookmarks: [],
            history: [ref],
            filter: .all,
            dictionaryID: nil,
            sort: .recent,
            dictionaryNames: [:]
        )

        XCTAssertEqual(presentation.entries.first?.dictionaryName, "已移除的词典")
    }
}
