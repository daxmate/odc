import XCTest
@testable import ODictKit

final class LibraryStoreTests: XCTestCase {
    func testSaveAndLoadRoundTrip() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }

        let item = LibraryItem(
            displayName: "demo",
            sourceName: "demo",
            sourceBytes: 4096,
            sourceHash: "abc123",
            importedAt: Date(timeIntervalSince1970: 1_760_000_000)
        )
        let state = LibraryState(items: [item], lastOpenedID: item.id)

        try LibraryStore.save(state, to: directory)

        XCTAssertEqual(LibraryStore.load(from: directory), state)
        XCTAssertEqual(LibraryStore.load(from: directory).lastOpenedID, item.id)
    }

    func testMissingFileLoadsAnEmptyLibrary() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }

        let state = LibraryStore.load(from: directory)
        XCTAssertTrue(state.items.isEmpty)
        XCTAssertNil(state.lastOpenedID)
    }

    func testCorruptFileIsQuarantinedInsteadOfThrown() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = LibraryStore.fileURL(in: directory)
        try Data("{not json".utf8).write(to: file)

        let state = LibraryStore.load(from: directory)

        XCTAssertTrue(state.items.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        let backups = try FileManager.default
            .contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.contains(".bak") }
        XCTAssertEqual(backups.count, 1, "the unreadable file is kept, never silently overwritten")
    }

    func testUnknownVersionIsQuarantined() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data(#"{"version":2,"items":[]}"#.utf8)
            .write(to: LibraryStore.fileURL(in: directory))

        let state = LibraryStore.load(from: directory)

        XCTAssertTrue(state.items.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: LibraryStore.fileURL(in: directory).path))
    }

    func testAddMergesTheSameSource() {
        var state = LibraryState()
        let first = LibraryItem(
            displayName: "A",
            sourceName: "dict.odc",
            sourceBytes: 10,
            sourceHash: "hash"
        )
        var second = LibraryItem(
            displayName: "B",
            sourceName: "dict.odc",
            sourceBytes: 10,
            sourceHash: "hash"
        )
        second.importedAt = first.importedAt

        LibraryStore.add(first, to: &state)
        LibraryStore.add(second, to: &state)

        XCTAssertEqual(state.items.count, 1)
        XCTAssertEqual(state.items[0].id, first.id, "the copy on disk keeps its identifier")
        XCTAssertEqual(state.items[0].displayName, "B")
    }

    func testAddKeepsDistinctSources() {
        var state = LibraryState()
        LibraryStore.add(
            LibraryItem(displayName: "A", sourceName: "a.odc", sourceBytes: 10, sourceHash: "a"),
            to: &state
        )
        LibraryStore.add(
            LibraryItem(displayName: "B", sourceName: "b.odc", sourceBytes: 10, sourceHash: "b"),
            to: &state
        )

        XCTAssertEqual(state.items.count, 2)
    }

    func testRemoveClearsTheLastOpenedEntry() {
        let item = LibraryItem(displayName: "demo", sourceName: "demo", sourceBytes: 1)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        LibraryStore.remove(id: item.id, from: &state)

        XCTAssertTrue(state.items.isEmpty)
        XCTAssertNil(state.lastOpenedID)
    }

    func testMatchingNeedsTheSameSizeAndContent() {
        let item = LibraryItem(
            displayName: "dict",
            sourceName: "dict.odc",
            sourceBytes: 10,
            sourceHash: "hash-10"
        )

        XCTAssertTrue(
            item.matches(LibrarySourceDescriptor(name: "dict.odc", bytes: 10, contentHash: "hash-10"))
        )
        XCTAssertFalse(
            item.matches(LibrarySourceDescriptor(name: "dict.odc", bytes: 11, contentHash: "hash-10"))
        )
        XCTAssertFalse(
            item.matches(LibrarySourceDescriptor(name: "dict.odc", bytes: 10, contentHash: "other"))
        )
        XCTAssertTrue(
            item.matches(LibrarySourceDescriptor(name: "renamed.odc", bytes: 10, contentHash: "hash-10")),
            "a renamed copy of the same content is still the same dictionary"
        )
    }

    func testOpenTargetPointsAtTheDatabase() {
        let item = LibraryItem(
            displayName: "demo",
            storedName: "X.odc",
            sourceName: "demo",
            sourceBytes: 1
        )

        let target = LibraryStore.openTarget(for: item, in: URL(fileURLWithPath: "/tmp/library"))

        XCTAssertEqual(target.path, "/tmp/library/X.odc/odc.db")
    }

    func testPruneMissingDropsEntriesWhoseCopyIsGone() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let missing = LibraryItem(displayName: "gone", sourceName: "gone", sourceBytes: 1)
        var state = LibraryState(items: [missing], lastOpenedID: missing.id)

        let pruned = LibraryStore.pruneMissing(in: &state, directory: directory)

        XCTAssertEqual(pruned, [missing.id])
        XCTAssertTrue(state.items.isEmpty)
        XCTAssertNil(state.lastOpenedID)
    }

    func testFingerprintUsesNameSizeAndHeadOfFile() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appending(path: "sample.odc")
        try Data(repeating: 7, count: 4096).write(to: file)

        let descriptor = try LibraryFileFingerprint.descriptor(for: file)

        XCTAssertEqual(descriptor.name, "sample.odc")
        XCTAssertEqual(descriptor.bytes, 4096)
        XCTAssertEqual(descriptor.contentHash, try LibraryFileFingerprint.contentHash(of: file))
    }

    func testDeduplicateMovesExtraCopiesIntoTheTrash() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = try Data(contentsOf: Fixture.demoRepository.appending(path: "odc.db"))

        var state = LibraryState()
        var storedNames: [String] = []
        for _ in 0..<2 {
            let id = UUID()
            let storedName = "\(id.uuidString).odc"
            let copy = directory.appending(path: storedName, directoryHint: .isDirectory)
            try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
            try database.write(to: copy.appending(path: "odc.db"))
            storedNames.append(storedName)
            state.items.append(
                LibraryItem(
                    id: id,
                    displayName: "demo",
                    storedName: storedName,
                    sourceName: "demo",
                    sourceBytes: Int64(database.count)
                )
            )
        }
        state.lastOpenedID = state.items[0].id

        let report = try LibraryStore.deduplicateExistingCopies(in: directory, state: &state)

        XCTAssertEqual(report.removedCount, 1)
        XCTAssertEqual(report.removedStoredNames, [storedNames[1]])
        XCTAssertEqual(state.items.count, 1)
        XCTAssertEqual(state.items[0].storedName, storedNames[0], "the entry in use is kept")
        XCTAssertEqual(state.lastOpenedID, state.items[0].id)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: directory.appending(path: storedNames[0]).path)
        )
        let trash = try XCTUnwrap(report.trashDirectory)
        XCTAssertTrue(FileManager.default.fileExists(atPath: trash.appending(path: storedNames[1]).path))
    }

    func testDeduplicateLeavesDistinctCopiesAlone() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = try Data(contentsOf: Fixture.demoRepository.appending(path: "odc.db"))

        for extra in ["", "x"] {
            let copy = directory.appending(
                path: "\(UUID().uuidString).odc",
                directoryHint: .isDirectory
            )
            try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
            try (database + Data(extra.utf8)).write(to: copy.appending(path: "odc.db"))
        }

        var state = LibraryState()
        let report = try LibraryStore.deduplicateExistingCopies(in: directory, state: &state)

        XCTAssertTrue(report.isEmpty)
        XCTAssertEqual(
            try FileManager.default.contentsOfDirectory(atPath: directory.path)
                .filter { $0.hasSuffix(".odc") }
                .count,
            2
        )
    }
}
