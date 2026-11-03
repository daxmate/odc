import Foundation
import XCTest
@testable import ODictKit

/// End-to-end checks for the directory-shaped MDX import and the pack
/// conversion entry point.
@MainActor
final class MDXDirectoryImportTests: XCTestCase {
    func testImportedMDXIsRegisteredAsAnEditableDirectory() async throws {
        let container = try Fixture.temporaryDirectory()
        let store = DictionaryStore(container: container)
        defer { store.close() }

        // Copy the fixture so the conversion never writes beside the source.
        let source = container.appending(path: "sample.mdx", directoryHint: .notDirectory)
        try FileManager.default.copyItem(at: Fixture.sampleMDX, to: source)

        store.importMDX(url: source)
        try await waitUntil { store.documentURL != nil || store.errorMessage != nil }

        XCTAssertNil(store.errorMessage)
        let item = try XCTUnwrap(store.library.items.first)
        XCTAssertEqual(item.sourceIsDirectory, true, "MDX imports must be editable")
        XCTAssertTrue(store.isEditingAvailable)
        XCTAssertNotNil(store.makeEditorModel())
        XCTAssertEqual(store.documentName, "sample.mdx")

        // The library copy is a directory that holds odc.db.
        let copy = container.appending(path: item.storedName, directoryHint: .isDirectory)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: copy.appending(path: "odc.db").path),
            "the library copy must be a repository directory"
        )
    }

    func testConvertingAPackKeepsTheOriginalAndMakesAnEditableCopy() async throws {
        let container = try Fixture.temporaryDirectory()
        let store = DictionaryStore(container: container)
        defer { store.close() }

        let pack = container.appending(path: "source.odc", directoryHint: .notDirectory)
        try FileManager.default.copyItem(at: Fixture.samplePack, to: pack)
        let before = try Data(contentsOf: pack)

        store.openDocument(url: pack)
        try await waitUntil { store.documentURL != nil || store.errorMessage != nil }
        XCTAssertNil(store.errorMessage)
        let original = try XCTUnwrap(store.library.items.first)
        XCTAssertEqual(original.sourceIsDirectory, false, "a pack stays read-only")

        await store.convertPackToEditable(itemID: original.id)
        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.library.items.count, 2, "the pack stays in the library")

        let copy = try XCTUnwrap(store.library.items.first { $0.id != original.id })
        XCTAssertEqual(copy.sourceIsDirectory, true)
        XCTAssertEqual(copy.displayName, original.displayName)
        XCTAssertTrue(store.isEditingAvailable)
        XCTAssertNotNil(store.makeEditorModel())

        XCTAssertEqual(try Data(contentsOf: pack), before, "the pack must not change")
        let stored = container.appending(path: copy.storedName, directoryHint: .isDirectory)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: stored.appending(path: "odc.db").path)
        )
    }

    /// Poll until `condition` holds; the store imports on a detached task.
    private func waitUntil(
        timeout: Duration = .seconds(20),
        _ condition: () -> Bool
    ) async throws {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTFail("condition not met within \(timeout)")
    }
}

