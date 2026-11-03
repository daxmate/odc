import Foundation
import ODictCore
import XCTest

/// Imports only `ODictCore` on purpose: the binding must stand alone without
/// the application layer that lives in `ODictKit`.
@MainActor
final class ODictCoreStandaloneTests: XCTestCase {
    /// The demo repository is committed under the ODictKit test target; reuse
    /// that fixture rather than committing a second copy of the database.
    private var demoRepository: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appending(path: "ODictKitTests/Fixtures/demo", directoryHint: .isDirectory)
    }

    /// The fixture is version controlled; opening it in place would let the C
    /// core touch the file, so every test opens a private copy.
    private func makeCopy() throws -> URL {
        let container = FileManager.default.temporaryDirectory
            .appending(path: "ODictCoreTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: container)
        }
        let destination = container.appending(path: "demo.odc", directoryHint: .notDirectory)
        try FileManager.default.copyItem(at: demoRepository, to: destination)
        return destination
    }

    func testOpenAndSearchFormASelfContainedLoop() throws {
        let copy = try makeCopy()
        let dictionary = try OdcDictionary(url: copy)
        XCTAssertTrue(dictionary.isOpen)

        let hits = try dictionary.search("banana")
        XCTAssertEqual(hits.first?.key, "banana")

        let entry = try dictionary.entry(for: "banana")
        XCTAssertEqual(entry.key, "banana")
    }
}
