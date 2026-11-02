import XCTest
@testable import ODictKit

@MainActor
final class SearchHistoryStoreTests: XCTestCase {
    private static let storageKey = "history"

    private func makeSuite() -> (defaults: UserDefaults, name: String) {
        let name = "SearchHistoryStoreTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return (defaults, name)
    }

    private func tearDownSuite(_ defaults: UserDefaults, _ name: String) {
        defaults.removePersistentDomain(forName: name)
    }

    func testBlankQueriesAreIgnored() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)

        store.record("")
        store.record("   ")
        store.record("\n\t")

        XCTAssertTrue(store.items.isEmpty)
        XCTAssertTrue(defaults.stringArray(forKey: Self.storageKey)?.isEmpty ?? true)
    }

    func testRecordTrimsAndKeepsNewestFirst() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)

        store.record("  apple ")
        store.record("banana")

        XCTAssertEqual(store.items, ["banana", "apple"])
    }

    func testRecordDeduplicatesCaseInsensitively() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)

        store.record("apple")
        store.record("Banana")
        store.record("APPLE")

        XCTAssertEqual(store.items, ["APPLE", "Banana"])
    }

    func testRecordTruncatesToLimit() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey, limit: 3)

        for term in ["one", "two", "three", "four"] {
            store.record(term)
        }

        XCTAssertEqual(store.items, ["four", "three", "two"])
        XCTAssertEqual(defaults.stringArray(forKey: Self.storageKey), ["four", "three", "two"])
    }

    func testHistoryPersistsAcrossInstances() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }

        let first = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)
        first.record("alpha")
        first.record("beta")

        let second = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)
        XCTAssertEqual(second.items, ["beta", "alpha"])
    }

    func testRemoveDropsMatchingEntryCaseInsensitively() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)
        store.record("alpha")
        store.record("beta")

        store.remove("ALPHA")

        XCTAssertEqual(store.items, ["beta"])
        XCTAssertEqual(defaults.stringArray(forKey: Self.storageKey), ["beta"])
    }

    func testClearRemovesEverything() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey)
        store.record("alpha")

        store.clear()

        XCTAssertTrue(store.items.isEmpty)
        XCTAssertEqual(defaults.stringArray(forKey: Self.storageKey), [])
    }

    func testInitSanitizesStoredValues() {
        let (defaults, name) = makeSuite()
        defer { tearDownSuite(defaults, name) }
        defaults.set(["", "  ", "alpha", "ALPHA", "beta", "gamma", "delta"], forKey: Self.storageKey)

        let store = SearchHistoryStore(userDefaults: defaults, storageKey: Self.storageKey, limit: 3)

        XCTAssertEqual(store.items, ["alpha", "beta", "gamma"])
    }
}
