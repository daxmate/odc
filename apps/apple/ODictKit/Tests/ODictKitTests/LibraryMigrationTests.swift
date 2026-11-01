import XCTest
@testable import ODictKit

/// Startup migration: copies living outside the library folder move in, and
/// nothing is lost when a copy cannot be read.
final class LibraryMigrationTests: XCTestCase {
    // MARK: - Moving in

    func testExternalCopyIsCopiedInAndTheEntryFollows() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = try fixture.makeExternalRepository(name: "LDOCE6++ En-Cn V2-19-1.odc")
        let item = fixture.item(id: UUID(), storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        let report = LibraryStore.migrateExternalCopies(
            in: fixture.library,
            state: &state,
            verify: LibraryStore.verifyRepository
        )

        XCTAssertEqual(report.migrated.count, 1)
        XCTAssertTrue(report.issues.isEmpty)
        XCTAssertEqual(state.items.count, 1)
        XCTAssertEqual(state.items[0].storedName, "\(item.id.uuidString).odc")
        XCTAssertEqual(state.items[0].id, item.id)
        XCTAssertTrue(LibraryStore.isStored(state.items[0], in: fixture.library))
        let copy = LibraryStore.copyDirectory(for: state.items[0], in: fixture.library)
        XCTAssertEqual(copy.path, fixture.library.appending(path: "\(item.id.uuidString).odc").path)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: copy.appending(path: "odc.db").path)
        )
        // The original stays where it was: migrating never deletes anything.
        XCTAssertTrue(FileManager.default.fileExists(atPath: external.appending(path: "odc.db").path))
    }

    func testSecondRunDoesNothing() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = try fixture.makeExternalRepository(name: "Dict.odc")
        let item = fixture.item(storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        _ = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)
        let second = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertTrue(second.isEmpty)
        XCTAssertEqual(try fixture.entries(in: fixture.library).count, 1)
        XCTAssertEqual(state.items[0].storedName, "\(item.id.uuidString).odc")
    }

    func testStandaloneFileIsWrappedIntoARepositoryDirectory() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let file = fixture.external.appending(path: "Single.odc")
        try FixtureLayout.database.write(to: file)
        let item = fixture.item(storedName: file.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertTrue(report.issues.isEmpty)
        let copy = LibraryStore.copyDirectory(for: state.items[0], in: fixture.library)
        XCTAssertTrue(FileManager.default.fileExists(atPath: copy.appending(path: "odc.db").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
    }

    // MARK: - Name clashes

    func testTakenNameGetsAFreshIdentifier() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = try fixture.makeExternalRepository(name: "Dict.odc")
        let id = UUID()
        let item = fixture.item(id: id, storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: id)
        // Something unreadable already sits under the entry's own name.
        let taken = fixture.library.appending(path: "\(id.uuidString).odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: taken, withIntermediateDirectories: true)
        try Data("not a database".utf8).write(to: taken.appending(path: "odc.db"))

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertTrue(report.issues.isEmpty)
        let migrated = try XCTUnwrap(state.items.first)
        XCTAssertNotEqual(migrated.id, id)
        XCTAssertEqual(migrated.storedName, "\(migrated.id.uuidString).odc")
        XCTAssertEqual(state.lastOpenedID, migrated.id)
        // The occupied folder is left alone.
        let occupied = try Data(contentsOf: taken.appending(path: "odc.db"))
        XCTAssertEqual(occupied, Data("not a database".utf8))
    }

    func testHalfFinishedCopyIsReusedInsteadOfDuplicated() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = try fixture.makeExternalRepository(name: "Dict.odc")
        let id = UUID()
        let item = fixture.item(id: id, storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: id)
        // A previous run copied but died before saving the index.
        let existing = fixture.library.appending(path: "\(id.uuidString).odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: existing, withIntermediateDirectories: true)
        try FixtureLayout.database.write(to: existing.appending(path: "odc.db"))

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertTrue(report.issues.isEmpty)
        XCTAssertEqual(state.items[0].id, id)
        XCTAssertEqual(try fixture.entries(in: fixture.library).count, 1)
    }

    // MARK: - Failures keep the entry

    func testMissingSourceKeepsTheEntryAndExplainsWhy() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let gone = fixture.external.appending(path: "Gone.odc")
        let item = fixture.item(storedName: gone.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertTrue(report.migrated.isEmpty)
        XCTAssertEqual(report.issues.count, 1)
        XCTAssertEqual(report.issues[0].id, item.id)
        XCTAssertTrue(report.issues[0].reason.contains("原件已不在原位置"), report.issues[0].reason)
        XCTAssertEqual(state.items, [item])
    }

    func testUnusableCopyIsRejectedAndTheEntryIsKept() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = fixture.external.appending(path: "Broken.odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: external, withIntermediateDirectories: true)
        try Data("not a database".utf8).write(to: external.appending(path: "odc.db"))
        let item = fixture.item(storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertEqual(report.issues.count, 1)
        XCTAssertTrue(report.issues[0].reason.contains("无法打开"), report.issues[0].reason)
        XCTAssertEqual(state.items, [item])
        // A copy that does not open never lingers in the library folder.
        XCTAssertTrue(try fixture.entries(in: fixture.library).isEmpty)
    }

    func testUnreadableSourceIsReportedNotThrown() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let external = try fixture.makeExternalRepository(name: "Locked.odc")
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o000],
            ofItemAtPath: external.appending(path: "odc.db").path
        )
        addTeardownBlock {
            try? FileManager.default.setAttributes(
                [.posixPermissions: 0o644],
                ofItemAtPath: external.appending(path: "odc.db").path
            )
        }
        let item = fixture.item(storedName: external.path)
        var state = LibraryState(items: [item], lastOpenedID: item.id)

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertEqual(report.issues.count, 1)
        XCTAssertEqual(state.items, [item])
        XCTAssertTrue(try fixture.entries(in: fixture.library).isEmpty)
    }

    // MARK: - Detection

    func testEntriesInsideTheLibraryAreNotCandidates() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let inside = fixture.library.appending(path: "inside.odc", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: inside, withIntermediateDirectories: true)
        let item = fixture.item(storedName: "inside.odc")

        XCTAssertTrue(LibraryStore.isStored(item, in: fixture.library))

        var state = LibraryState(items: [item])
        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)
        XCTAssertTrue(report.isEmpty)
    }

    func testEntryPointingAtTheLibraryItselfIsNotCopied() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        var state = LibraryState(items: [fixture.item(storedName: ".")])

        let report = LibraryStore.migrateExternalCopies(in: fixture.library, state: &state)

        XCTAssertEqual(report.issues.count, 1)
        XCTAssertTrue(report.migrated.isEmpty)
        XCTAssertTrue(try fixture.entries(in: fixture.library).isEmpty)
    }

    func testEscapingRelativeNameIsTreatedAsExternal() throws {
        let fixture = try FixtureLayout()
        addTeardownBlock { fixture.cleanup() }
        let item = fixture.item(storedName: "../outside/escape.odc")

        XCTAssertFalse(LibraryStore.isStored(item, in: fixture.library))
    }
}

/// Library folder plus a sibling folder standing in for the reader's Documents.
private struct FixtureLayout {
    static let database = Data("SQLite format 3\0".utf8) + Data(repeating: 0, count: 512)

    let root: URL
    let library: URL
    let external: URL

    init() throws {
        root = try Fixture.temporaryDirectory()
        library = root.appending(path: "library", directoryHint: .isDirectory)
        external = root.appending(path: "external", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: library, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: external, withIntermediateDirectories: true)
    }

    func cleanup() {
        try? FileManager.default.removeItem(at: root)
    }

    func item(id: UUID = UUID(), storedName: String) -> ODictKit.LibraryItem {
        LibraryItem(
            id: id,
            displayName: "LDOCE6++ En-Cn V2-19-1",
            storedName: storedName,
            sourceName: "LDOCE6++.mdx",
            sourceBytes: 1024
        )
    }

    @discardableResult
    func makeExternalRepository(name: String) throws -> URL {
        let copy = external.appending(path: name, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
        try Self.database.write(to: copy.appending(path: "odc.db"))
        return copy
    }

    func entries(in directory: URL) throws -> [String] {
        try FileManager.default
            .contentsOfDirectory(atPath: directory.path)
            .filter { !$0.hasPrefix(".") }
    }
}
