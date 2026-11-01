import XCTest
@testable import ODictKit

/// End-to-end checks for the two complaints that started this work: a relaunch
/// forgetting the imported dictionary, and a second import leaving a second copy.
@MainActor
final class DictionaryLibraryTests: XCTestCase {
    func testImportedDictionaryReopensAfterRelaunch() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let first = DictionaryStore(container: container)
        first.open(url: Fixture.demoRepository)
        try await waitUntil { first.documentURL != nil }
        let copy = try XCTUnwrap(first.documentURL)
        XCTAssertEqual(first.library.items.count, 1)

        let second = DictionaryStore(container: container)
        second.restore()
        await second.awaitRestore()

        XCTAssertEqual(second.documentURL?.lastPathComponent, copy.lastPathComponent)
        XCTAssertEqual(second.documentName, "demo")
        XCTAssertEqual(second.library.items.count, 1)

        second.setQuery("app")
        try await waitUntil { !second.hits.isEmpty }
        XCTAssertEqual(second.hits.map(\.key), ["apple", "application"])
    }

    func testImportingTheSameFileTwiceKeepsOneCopy() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let store = DictionaryStore(container: container)
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }
        let copy = try XCTUnwrap(store.documentURL)

        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.importSummary != nil }

        let copies = try FileManager.default
            .contentsOfDirectory(atPath: container.path)
            .filter { $0.hasSuffix(".odc") }
        XCTAssertEqual(copies.count, 1, "the second import must not copy the file again")
        XCTAssertEqual(store.library.items.count, 1)
        XCTAssertEqual(store.documentURL?.lastPathComponent, copy.lastPathComponent)
        XCTAssertEqual(store.importSummary, "这本词典已在库中，已直接打开。")
    }

    func testImportAdoptsACopyThatPredatesTheLibrary() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }
        let picked = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: picked) }

        // What an older build left behind: a copy directory and no library.json.
        let database = try Data(contentsOf: Fixture.demoRepository.appending(path: "odc.db"))
        let legacyID = UUID()
        let legacyName = "\(legacyID.uuidString).odc"
        let legacy = container.appending(path: legacyName, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: legacy, withIntermediateDirectories: true)
        try database.write(to: legacy.appending(path: "odc.db"))

        let file = picked.appending(path: "demo.odc")
        try database.write(to: file)

        let store = DictionaryStore(container: container)
        store.open(url: file)
        try await waitUntil { store.documentURL != nil }

        XCTAssertEqual(store.documentURL?.lastPathComponent, legacyName)
        XCTAssertEqual(store.library.items.count, 1)
        XCTAssertEqual(store.importSummary, "这本词典已在库中，已直接打开。")
        let copies = try FileManager.default
            .contentsOfDirectory(atPath: container.path)
            .filter { $0.hasSuffix(".odc") }
        XCTAssertEqual(copies.count, 1, "adopting must not write a second copy")
    }

    func testRestoreAdoptsCopiesThatPredateTheLibrary() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }
        let database = try Data(contentsOf: Fixture.demoRepository.appending(path: "odc.db"))

        var storedNames: [String] = []
        var createdAt: [String: Date] = [:]
        for _ in 0..<2 {
            let id = UUID()
            let copy = container.appending(
                path: "\(id.uuidString).odc",
                directoryHint: .isDirectory
            )
            try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
            try database.write(to: copy.appending(path: "odc.db"))
            storedNames.append(copy.lastPathComponent)
            let created = try copy.resourceValues(forKeys: [.creationDateKey]).creationDate
            createdAt[copy.lastPathComponent] = created
        }

        let store = DictionaryStore(container: container)
        store.restore()
        await store.awaitRestore()

        XCTAssertNil(store.documentURL, "adopted copies are not opened without the reader picking one")
        XCTAssertEqual(store.library.items.count, 2)
        XCTAssertEqual(Set(store.library.items.map(\.storedName)), Set(storedNames))
        for item in store.library.items {
            XCTAssertTrue(item.displayName.hasPrefix("未命名词典 "), item.displayName)
            XCTAssertEqual(item.displayName.count, "未命名词典 ".count + 8)
            XCTAssertTrue(item.sourceName.isEmpty)
            XCTAssertEqual(item.sourceBytes, Int64(database.count))
            let expected = try XCTUnwrap(createdAt[item.storedName])
            XCTAssertEqual(
                item.importedAt.timeIntervalSince1970,
                expected.timeIntervalSince1970,
                accuracy: 1,
                "the directory creation date is the import date"
            )
        }
        XCTAssertEqual(LibraryStore.load(from: container).items.count, 2)

        let second = DictionaryStore(container: container)
        second.restore()
        await second.awaitRestore()

        XCTAssertEqual(second.library.items.count, 2, "adoption is idempotent")
        XCTAssertEqual(LibraryStore.load(from: container).items.count, 2)
    }

    func testReimportingAnAdoptedCopyReplacesThePlaceholderName() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }
        let picked = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: picked) }
        let database = try Data(contentsOf: Fixture.demoRepository.appending(path: "odc.db"))

        let id = UUID()
        let copy = container.appending(path: "\(id.uuidString).odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
        try database.write(to: copy.appending(path: "odc.db"))

        let store = DictionaryStore(container: container)
        store.restore()
        await store.awaitRestore()
        XCTAssertTrue(try XCTUnwrap(store.library.items.first).isPlaceholder)

        let file = picked.appending(path: "demo.odc")
        try database.write(to: file)
        store.open(url: file)
        try await waitUntil { store.importSummary != nil }

        XCTAssertEqual(store.library.items.count, 1, "the adopted copy is upgraded, not duplicated")
        let item = try XCTUnwrap(store.library.items.first)
        XCTAssertFalse(item.isPlaceholder)
        XCTAssertEqual(item.displayName, "demo.odc")
        XCTAssertEqual(item.sourceName, "demo.odc")
        XCTAssertEqual(item.storedName, copy.lastPathComponent)
        XCTAssertEqual(store.documentURL?.lastPathComponent, copy.lastPathComponent)
        XCTAssertEqual(
            try FileManager.default.contentsOfDirectory(atPath: container.path)
                .filter { $0.hasSuffix(".odc") }
                .count,
            1
        )
    }

    func testRemovingAnItemDeletesItsCopyAndEntry() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let store = DictionaryStore(container: container)
        store.open(url: Fixture.demoRepository)
        try await waitUntil { store.documentURL != nil }
        let copy = try XCTUnwrap(store.documentURL)
        let id = try XCTUnwrap(store.library.items.first?.id)

        store.removeItem(id: id)

        XCTAssertNil(store.documentURL)
        XCTAssertTrue(store.library.items.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: copy.path))
        XCTAssertTrue(LibraryStore.load(from: container).items.isEmpty)
    }

    func testRestoreDropsAnEntryWhoseCopyDisappeared() async throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let first = DictionaryStore(container: container)
        first.open(url: Fixture.demoRepository)
        try await waitUntil { first.documentURL != nil }
        let copy = try XCTUnwrap(first.documentURL)
        try FileManager.default.removeItem(at: copy)

        let second = DictionaryStore(container: container)
        second.restore()
        await second.awaitRestore()

        XCTAssertNil(second.documentURL)
        XCTAssertTrue(second.library.items.isEmpty)
        XCTAssertTrue(LibraryStore.load(from: container).items.isEmpty)
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        _ condition: () -> Bool
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
}
