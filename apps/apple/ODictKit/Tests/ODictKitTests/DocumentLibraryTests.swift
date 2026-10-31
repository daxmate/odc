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

    func testRemovingAMissingCopyIsIgnored() throws {
        let missing = URL(fileURLWithPath: "/nonexistent/ODictKitTests/copy.odc")
        DocumentLibrary.removeCopy(at: missing)
    }
}
