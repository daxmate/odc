import XCTest
@testable import ODictKit

@MainActor
final class OdcDictionaryTests: XCTestCase {
    func testOpeningAMissingPathFails() {
        let url = URL(fileURLWithPath: "/nonexistent/ODictKitTests/missing.odc")
        XCTAssertThrowsError(try OdcDictionary(url: url)) { error in
            guard case OdcError.openFailed(let message) = error else {
                return XCTFail("expected openFailed, got \(error)")
            }
            XCTAssertFalse(message.isEmpty)
        }
    }

    func testLimitOutsideTheABIRangeIsRejected() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        XCTAssertThrowsError(try dictionary.search("app", limit: 0)) { error in
            XCTAssertEqual(error as? OdcError, .invalidLimit(0))
        }
        XCTAssertThrowsError(try dictionary.search("app", limit: 201)) { error in
            XCTAssertEqual(error as? OdcError, .invalidLimit(201))
        }
    }

    func testPrefixSearchReturnsHitsInKeyOrder() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        let hits = try dictionary.search("app")
        XCTAssertEqual(hits.map(\.key), ["apple", "application"])
        XCTAssertEqual(hits.map(\.display), ["apple", "application"])
    }

    func testSearchWithoutMatchesIsEmpty() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        XCTAssertEqual(try dictionary.search("zzz"), [])
    }

    func testLookupReturnsStructuredEntry() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        let entry = try dictionary.entry(for: "banana")
        XCTAssertEqual(entry.key, "banana")
        XCTAssertEqual(entry.phonetic, ["bəˈnɑː.nə"])
        XCTAssertEqual(entry.defs.count, 1)
        XCTAssertEqual(entry.defs.first?.pos, "noun")
        XCTAssertEqual(entry.defs.first?.labels, ["informal"])
        XCTAssertEqual(entry.rawHTML, "<b>banana</b>")
        XCTAssertEqual(entry.extra, [:])
    }

    func testLookupPreservesExtraFields() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        let entry = try dictionary.entry(for: "application")
        XCTAssertEqual(entry.extra["source"], .string("fixture"))
        XCTAssertEqual(entry.extra["revision"], .number(1))
    }

    func testMissingLookupFails() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        XCTAssertThrowsError(try dictionary.entry(for: "zzz")) { error in
            guard case OdcError.entryFailed(let message) = error else {
                return XCTFail("expected entryFailed, got \(error)")
            }
            XCTAssertTrue(message.contains("zzz"))
        }
    }

    func testCloseIsIdempotentAndRejectsLaterCalls() throws {
        let dictionary = try OdcDictionary(url: Fixture.demoRepository)
        dictionary.close()
        dictionary.close()
        XCTAssertFalse(dictionary.isOpen)
        XCTAssertThrowsError(try dictionary.search("app")) { error in
            XCTAssertEqual(error as? OdcError, .closed)
        }
    }
}
