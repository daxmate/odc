import XCTest
@testable import ODictKit

@MainActor
final class MultiDictionaryTests: XCTestCase {
    private func makeContainer() throws -> URL {
        let container = try Fixture.temporaryDirectory()
        addTeardownBlock { try? FileManager.default.removeItem(at: container) }
        return container
    }

    /// Copies the committed fixture once per item, so every dictionary has its
    /// own repository directory and its own library identifier.
    @discardableResult
    private func makeLibrary(in container: URL, count: Int) throws -> [ODictKit.LibraryItem] {
        var items: [ODictKit.LibraryItem] = []
        for index in 0..<count {
            let id = UUID()
            let copy = container.appending(
                path: "\(id.uuidString).odc",
                directoryHint: .isDirectory
            )
            try FileManager.default.copyItem(at: Fixture.demoRepository, to: copy)
            items.append(
                ODictKit.LibraryItem(
                    id: id,
                    displayName: "词典\(index + 1)",
                    storedName: "\(id.uuidString).odc",
                    sourceName: "demo-\(index + 1)",
                    sourceBytes: 4096
                )
            )
        }
        try LibraryStore.save(LibraryState(items: items), to: container)
        return items
    }

    private func makeStore(container: URL) async throws -> DictionaryStore {
        let store = DictionaryStore(container: container)
        store.restore()
        await store.awaitRestore()
        return store
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

    private func open(_ item: ODictKit.LibraryItem, in store: DictionaryStore) async throws {
        store.open(item: item.id)
        try await waitUntil { store.isOpen(item.id) || store.errorMessage != nil }
    }

    func testOpeningASecondDictionaryKeepsTheFirstOneOpen() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        XCTAssertEqual(store.documentName, items[0].displayName)

        try await open(items[1], in: store)

        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.openDictionaries.map(\.id), [items[0].id, items[1].id])
        XCTAssertEqual(store.activeDictionaryID, items[1].id)
        XCTAssertEqual(store.documentName, items[1].displayName)
        XCTAssertEqual(store.documentURL?.lastPathComponent, items[1].storedName)
    }

    func testOpeningTheSameItemTwiceKeepsOneEntry() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 1)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        store.open(item: items[0].id)
        try await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(store.openDictionaries.count, 1)
    }

    func testSearchReturnsOneGroupPerOpenDictionary() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        store.setQuery("app")
        try await waitUntil { store.searchGroups.count == 2 }

        XCTAssertEqual(store.searchGroups.map(\.dictionaryID), [items[0].id, items[1].id])
        XCTAssertEqual(store.searchGroups.map(\.name), [items[0].displayName, items[1].displayName])
        for group in store.searchGroups {
            XCTAssertEqual(group.hits.map(\.key), ["apple", "application"])
        }
        // The sidebar list still follows the active dictionary.
        XCTAssertEqual(store.hits.map(\.key), ["apple", "application"])
    }

    func testPickingAHitFromAnotherDictionarySwitchesTheActiveOne() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)
        XCTAssertEqual(store.activeDictionaryID, items[1].id)

        store.setQuery("app")
        try await waitUntil { store.searchGroups.count == 2 }

        await store.select(key: "application", in: items[0].id)

        XCTAssertEqual(store.activeDictionaryID, items[0].id)
        XCTAssertEqual(store.entry?.key, "application")
        XCTAssertEqual(store.documentName, items[0].displayName)
        XCTAssertEqual(store.recentEntries.first?.dictionaryID, items[0].id.uuidString)
    }

    func testBookmarksFollowTheTargetDictionary() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "apple", in: items[0].id)
        store.toggleBookmark()

        XCTAssertEqual(store.bookmarkedEntries.count, 1)
        XCTAssertEqual(store.bookmarkedEntries.first?.dictionaryID, items[0].id.uuidString)
        XCTAssertTrue(store.isBookmarked(key: "apple"))

        // The same headword in the other dictionary is a different reference.
        store.activateDictionary(id: items[1].id)
        XCTAssertNil(store.entry)
        await store.select(key: "apple")
        XCTAssertFalse(store.isBookmarked(key: "apple"))

        store.toggleBookmark()
        XCTAssertEqual(Set(store.bookmarkedEntries.map(\.dictionaryID)), [
            items[0].id.uuidString,
            items[1].id.uuidString,
        ])
    }

    func testClosingTheActiveDictionaryFallsBackToAnotherOne() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)
        XCTAssertEqual(store.activeDictionaryID, items[1].id)

        store.closeLibraryItem(id: items[1].id)

        XCTAssertEqual(store.activeDictionaryID, items[0].id)
        XCTAssertEqual(store.documentName, items[0].displayName)
        XCTAssertNil(store.errorMessage)
        // Closing keeps the copy, so the item can be opened again.
        XCTAssertEqual(store.library.items.count, 2)

        store.closeLibraryItem(id: items[0].id)

        XCTAssertNil(store.activeDictionaryID)
        XCTAssertNil(store.documentURL)
        XCTAssertTrue(store.hits.isEmpty)
    }

    func testTheNinthDictionaryIsRefusedWithAMessage() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(
            in: container,
            count: DictionaryStore.maximumOpenDictionaries + 1
        )
        let store = try await makeStore(container: container)
        defer { store.close() }

        for item in items.prefix(DictionaryStore.maximumOpenDictionaries) {
            try await open(item, in: store)
        }
        XCTAssertEqual(
            store.openDictionaries.count,
            DictionaryStore.maximumOpenDictionaries
        )

        store.open(item: items[DictionaryStore.maximumOpenDictionaries].id)
        try await waitUntil { store.errorMessage != nil }

        XCTAssertEqual(
            store.openDictionaries.count,
            DictionaryStore.maximumOpenDictionaries
        )
        XCTAssertEqual(store.openDictionaries.map(\.id).last, items[7].id)
    }

    func testRemovingAnOpenDictionaryClosesItAndFallsBack() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        store.removeItem(id: items[1].id)

        XCTAssertFalse(store.isOpen(items[1].id))
        XCTAssertEqual(store.activeDictionaryID, items[0].id)
        XCTAssertEqual(store.library.items.count, 1)
    }

    // MARK: - Persisted open set

    private func saveOpenSet(
        _ items: [ODictKit.LibraryItem],
        open: [UUID],
        active: UUID?,
        in container: URL
    ) throws {
        try LibraryStore.save(
            LibraryState(items: items, lastOpenedID: active, openItemIDs: open),
            to: container
        )
    }

    func testOpeningASecondDictionaryRecordsTheOpenSet() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        let saved = LibraryStore.load(from: container)
        XCTAssertEqual(saved.openItemIDs, [items[0].id, items[1].id])
        XCTAssertEqual(saved.lastOpenedID, items[1].id)
    }

    func testClosingADictionaryDropsItFromTheSavedOpenSet() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        store.closeLibraryItem(id: items[1].id)

        let saved = LibraryStore.load(from: container)
        XCTAssertEqual(saved.openItemIDs, [items[0].id])
        XCTAssertEqual(saved.lastOpenedID, items[0].id)
        XCTAssertEqual(store.activeDictionaryID, items[0].id)
    }

    func testSwitchingTheActiveDictionaryIsSaved() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        store.activateDictionary(id: items[0].id)

        let saved = LibraryStore.load(from: container)
        XCTAssertEqual(saved.lastOpenedID, items[0].id)
        XCTAssertEqual(saved.openItemIDs, [items[0].id, items[1].id])
    }

    func testRestoreReopensEveryDictionaryInOpenOrder() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 3)
        try saveOpenSet(
            items,
            open: items.map(\.id),
            active: items[1].id,
            in: container
        )

        let store = try await makeStore(container: container)
        defer { store.close() }

        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.openDictionaries.map(\.id), items.map(\.id))
        XCTAssertEqual(store.activeDictionaryID, items[1].id)
    }

    func testRestoreSkipsACopyThatIsGoneAndKeepsTheRest() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 3)
        let missing = LibraryStore.openTarget(for: items[1], in: container)
        try FileManager.default.removeItem(at: missing.deletingLastPathComponent())
        try saveOpenSet(
            items,
            open: items.map(\.id),
            active: items[1].id,
            in: container
        )

        let store = try await makeStore(container: container)
        defer { store.close() }

        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.openDictionaries.map(\.id), [items[0].id, items[2].id])
        XCTAssertEqual(store.activeDictionaryID, items[2].id)

        let saved = LibraryStore.load(from: container)
        XCTAssertEqual(saved.openItemIDs, [items[0].id, items[2].id])
        XCTAssertEqual(saved.lastOpenedID, items[2].id)
        XCTAssertEqual(saved.items.map(\.id), [items[0].id, items[2].id])
    }

    func testRestoreFallsBackToTheEmptyStateWhenNoCopyOpens() async throws {
        let container = try makeContainer()
        let items = (0..<2).map { index in
            ODictKit.LibraryItem(
                displayName: "词典\(index + 1)",
                sourceName: "gone-\(index + 1)",
                sourceBytes: 1
            )
        }
        try saveOpenSet(items, open: items.map(\.id), active: items[0].id, in: container)

        let store = try await makeStore(container: container)
        defer { store.close() }

        XCTAssertNil(store.errorMessage)
        XCTAssertNil(store.documentURL)
        XCTAssertTrue(store.openDictionaries.isEmpty)
        XCTAssertNil(store.activeDictionaryID)
        XCTAssertEqual(LibraryStore.load(from: container).openItemIDs, [])
    }

    func testRestoreKeepsOnlyTheNewestEightDictionaries() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(
            in: container,
            count: DictionaryStore.maximumOpenDictionaries + 1
        )
        try saveOpenSet(items, open: items.map(\.id), active: items.last?.id, in: container)

        let store = try await makeStore(container: container)
        defer { store.close() }

        let expected = Array(items.suffix(DictionaryStore.maximumOpenDictionaries)).map(\.id)
        XCTAssertEqual(store.openDictionaries.map(\.id), expected)
        XCTAssertEqual(store.activeDictionaryID, items.last?.id)
    }

    // MARK: - Detail page sections

    func testEntrySectionsKeepEveryOpenDictionaryThatCarriesTheKey() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "apple", in: items[0].id)

        XCTAssertEqual(store.entrySections.map(\.dictionaryID), [items[0].id, items[1].id])
        XCTAssertEqual(store.entrySections.map(\.name), [items[0].displayName, items[1].displayName])
        XCTAssertEqual(store.entrySections.map(\.entry.key), ["apple", "apple"])
        XCTAssertNil(store.entrySections.first?.notice)
    }

    func testEntrySectionsPutTheActiveDictionaryFirst() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "apple", in: items[1].id)

        XCTAssertEqual(store.entrySections.map(\.dictionaryID), [items[1].id, items[0].id])
    }

    func testEntrySectionsSkipDictionariesWithoutTheKey() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "not-a-word-in-the-fixture", in: items[0].id)

        XCTAssertTrue(store.entrySections.isEmpty)
    }

    func testSectionLookupLeavesTheSelectionAlone() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "apple", in: items[0].id)
        let section = store.entrySection(for: "application", in: items[1].id)

        XCTAssertEqual(section?.entry.key, "application")
        XCTAssertEqual(section?.dictionaryID, items[1].id)
        XCTAssertEqual(store.selectedKey, "apple")
        XCTAssertEqual(store.activeDictionaryID, items[0].id)
        XCTAssertEqual(store.entrySections.map(\.entry.key), ["apple", "apple"])
    }

    func testClosingADictionaryDropsItsSection() async throws {
        let container = try makeContainer()
        let items = try makeLibrary(in: container, count: 2)
        let store = try await makeStore(container: container)
        defer { store.close() }

        try await open(items[0], in: store)
        try await open(items[1], in: store)

        await store.select(key: "apple", in: items[0].id)
        XCTAssertEqual(store.entrySections.count, 2)

        store.closeLibraryItem(id: items[1].id)

        XCTAssertEqual(store.entrySections.map(\.dictionaryID), [items[0].id])
    }
}
