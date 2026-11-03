import XCTest
@testable import ODictKit

/// Exercises the editor view model against the real C ABI and a copy of the
/// committed demo repository.
@MainActor
final class EditorIntegrationTests: XCTestCase {
    private func makeDictionary() throws -> OdcDictionary {
        let container = try Fixture.temporaryDirectory()
        addTeardownBlock {
            try? FileManager.default.removeItem(at: container)
        }
        let copy = try DocumentLibrary.copyDocument(at: Fixture.demoRepository, into: container)
        return try OdcDictionary(url: copy)
    }

    func testSaveAndDiscardRoundTrip() throws {
        let dictionary = try makeDictionary()
        defer { dictionary.close() }
        let model = DictionaryEditorModel(editing: dictionary, author: "integration")

        model.loadIfNeeded()
        XCTAssertFalse(model.rows.isEmpty)
        let key = try XCTUnwrap(model.rows.first?.key)
        model.select(key)
        let original = try XCTUnwrap(model.form)
        XCTAssertFalse(original.display.isEmpty)

        model.updateForm { $0.display = original.display + " edited" }
        XCTAssertTrue(model.save())
        XCTAssertTrue(model.hasChanges)
        XCTAssertEqual(model.rows.first { $0.key == key }?.change, .modified)
        XCTAssertTrue(model.diff?.fields.contains { $0.field == "display" && $0.changed } ?? false)

        XCTAssertTrue(model.discard())
        XCTAssertFalse(model.hasChanges)
        XCTAssertEqual(model.rows.first { $0.key == key }?.change, nil)
    }

    func testCommitClearsTheWorkingSet() throws {
        let dictionary = try makeDictionary()
        defer { dictionary.close() }
        var commits = 0
        let model = DictionaryEditorModel(editing: dictionary, author: "integration") {
            commits += 1
        }

        model.loadIfNeeded()
        let key = try XCTUnwrap(model.rows.first?.key)
        model.select(key)
        let original = try XCTUnwrap(model.form)
        model.updateForm { $0.display = original.display + " committed" }
        XCTAssertTrue(model.save())

        XCTAssertTrue(model.commit(message: "integration edit"))
        // Save and commit each notify the reader.
        XCTAssertEqual(commits, 2)
        XCTAssertTrue(model.committed)
        XCTAssertFalse(model.hasChanges)
        XCTAssertEqual(model.status.added + model.status.modified + model.status.removed, 0)
    }
}
