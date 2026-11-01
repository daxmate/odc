import XCTest
@testable import ODictKit

@MainActor
final class DocumentLibraryTests: XCTestCase {
    func testCopyingASingleFileContainer() throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let picked = container.appending(path: "picked.odc")
        try FileManager.default.copyItem(at: Fixture.demoRepository.appending(path: "odc.db"), to: picked)

        let copy = try DocumentLibrary.copyDocument(at: picked, into: container.appending(path: "store"))
        XCTAssertTrue(FileManager.default.fileExists(atPath: copy.appending(path: "odc.db").path))
        XCTAssertEqual(copy.pathExtension, "odc")
    }

    func testCopyingARepositoryDirectory() throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let copy = try DocumentLibrary.copyDocument(at: Fixture.demoRepository, into: container)
        XCTAssertTrue(FileManager.default.fileExists(atPath: copy.appending(path: "odc.db").path))
    }

    func testCopiedContainerOpensAndSearches() throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let copy = try DocumentLibrary.copyDocument(at: Fixture.demoRepository, into: container)
        let dictionary = try OdcDictionary(url: copy)
        XCTAssertEqual(try dictionary.search("ban").map(\.key), ["banana"])
    }

    func testApplicationSupportContainerIsTheLibraryFolder() throws {
        let container = try DocumentLibrary.applicationSupportContainer()

        XCTAssertEqual(container.lastPathComponent, "ODict")
        var isDirectory: ObjCBool = false
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: container.path, isDirectory: &isDirectory),
            "opening a dictionary must not depend on an earlier import having made the folder"
        )
        XCTAssertTrue(isDirectory.boolValue)
    }

    func testLibraryContainerCreatesAMissingFolder() throws {
        let support = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: support) }
        let expected = support.appending(path: "ODict", directoryHint: .isDirectory)
        XCTAssertFalse(FileManager.default.fileExists(atPath: expected.path))

        let created = try DocumentLibrary.libraryContainer(in: support)

        XCTAssertEqual(created, expected)
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: created.path, isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)
        // Idempotent: an existing folder is not an error.
        XCTAssertEqual(try DocumentLibrary.libraryContainer(in: support), expected)
    }

    func testRemovingAMissingCopyIsIgnored() throws {
        let missing = URL(fileURLWithPath: "/nonexistent/ODictKitTests/copy.odc")
        DocumentLibrary.removeCopy(at: missing)
    }
}
