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
