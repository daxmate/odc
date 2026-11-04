import Foundation
import Testing
@testable import ODictKit

private func temporaryDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appending(path: "odict-onboarding-\(UUID().uuidString)", directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

@Suite("Library start menu")
struct LibraryStartMenuTests {
    @Test("first run offers new, sample and open")
    func firstRun() {
        let state = LibraryStartState(hasOpenDictionary: false, libraryIsEmpty: true)
        #expect(state == .firstRun)
        #expect(state.actions == [.newDictionary, .sample, .open])
        #expect(state.showsRecentDictionaries == false)
    }

    @Test("a populated library keeps the create entry points")
    func populatedLibrary() {
        let state = LibraryStartState(hasOpenDictionary: false, libraryIsEmpty: false)
        #expect(state == .libraryNotEmpty)
        #expect(state.actions == [.newDictionary, .sample, .open])
        #expect(state.showsRecentDictionaries)
    }

    @Test("an open dictionary hides the start menu")
    func dictionaryOpen() {
        let state = LibraryStartState(hasOpenDictionary: true, libraryIsEmpty: true)
        #expect(state == .dictionaryOpen)
        #expect(state.actions.isEmpty)
    }
}

@Suite("Sample dictionary")
@MainActor
struct SampleDictionaryTests {
    @Test("ten entries carry a part of speech and an example")
    func content() {
        #expect(SampleDictionary.entries.count == 10)
        #expect(Set(SampleDictionary.entries.map(\.key)).count == SampleDictionary.entries.count)
        for entry in SampleDictionary.entries {
            #expect(!entry.defs.isEmpty, "\(entry.key) has no definitions")
            for definition in entry.defs {
                #expect(!definition.pos.isEmpty, "\(entry.key) has no part of speech")
                #expect(!definition.sense.isEmpty, "\(entry.key) has an empty sense")
                #expect(!definition.examples.isEmpty, "\(entry.key) has no example")
            }
        }
        #expect(SampleDictionary.entries.contains { $0.rawHTML?.isEmpty == false })
    }

    @Test("populating a new repository writes ten readable entries")
    func populate() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "sample.odc", directoryHint: .notDirectory)
        let dictionary = try OdcDictionary.create(at: url)
        defer { dictionary.close() }

        try SampleDictionary.populate(dictionary)

        let entries = try dictionary.entriesPage(limit: 50)
        #expect(entries.count == SampleDictionary.entries.count)
        let apple = try dictionary.entry(for: "apple")
        #expect(apple.defs.first?.pos == "noun")
        #expect(apple.defs.first?.examples.first == "She ate an apple after lunch.")
        #expect(apple.rawHTML != nil)
        #expect(try dictionary.searchSubstring("ap").contains { $0.key == "apple" })
    }
}

@Suite("Dictionary creation")
@MainActor
struct DictionaryCreationTests {
    @Test("creating a repository yields an empty open dictionary")
    func createEmpty() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "empty.odc", directoryHint: .notDirectory)

        let dictionary = try OdcDictionary.create(at: url)
        defer { dictionary.close() }

        #expect(try dictionary.entriesPage(limit: 10).isEmpty)
        #expect(FileManager.default.fileExists(atPath: url.appending(path: "odc.db").path))
    }

    @Test("creating over an existing repository fails with a readable error")
    func createTwice() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "taken.odc", directoryHint: .notDirectory)
        let first = try OdcDictionary.create(at: url)
        first.close()

        do {
            _ = try OdcDictionary.create(at: url)
            Issue.record("expected a second create to fail")
        } catch let error as OdcError {
            guard case .initFailed(let detail) = error else {
                Issue.record("unexpected error: \(error)")
                return
            }
            #expect(detail.contains("already initialized"))
            #expect(error.localizedDescription.hasPrefix("无法新建词典："))
        }
    }

    @Test("the store rejects a blank name")
    func blankName() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = DictionaryStore(container: directory)

        #expect(throws: DictionaryCreationError.emptyName) {
            try store.createDictionary(named: "   ")
        }
        #expect(store.library.items.isEmpty)
    }

    @Test("the store registers and opens a new dictionary")
    func storeCreates() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = DictionaryStore(container: directory)

        let id = try store.createDictionary(named: "我的词典")

        #expect(store.library.items.count == 1)
        #expect(store.library.items.first?.displayName == "我的词典")
        #expect(store.activeDictionaryID == id)
        #expect(store.lastCreatedDictionaryID == id)
        #expect(store.documentURL != nil)
    }
}
