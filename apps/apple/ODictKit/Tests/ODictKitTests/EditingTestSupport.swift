import Foundation
@testable import ODictKit

/// In-memory `ODCEditing` used by the editor and importer tests.
@MainActor
final class FakeEditing: ODCEditing {
    var head: [String: DictionaryEntry]
    var working: [String: DictionaryEntry]
    var diffs: [String: EntryDiff] = [:]
    var failOnKeys: Set<String> = []
    /// Keys that live in storage but are never returned by `entriesPage`,
    /// i.e. entries sitting on pages the editor has not loaded yet.
    var unloadedKeys: Set<String> = []
    /// Keys whose lookup fails with a non-"not found" error.
    var lookupFailures: Set<String> = []
    var commitMessage: String?
    var commitAuthor: String?
    var discardCount = 0

    init(entries: [DictionaryEntry], changes: [(String, ChangeKind)] = []) {
        self.head = Dictionary(uniqueKeysWithValues: entries.map { ($0.key, $0) })
        self.working = self.head
        for (key, kind) in changes {
            switch kind {
            case .added:
                working[key] = DictionaryEntry(key: key, display: key)
            case .modified:
                if var entry = working[key] {
                    entry = DictionaryEntry(
                        key: entry.key,
                        display: entry.display + " (edited)",
                        phonetic: entry.phonetic,
                        defs: entry.defs,
                        rawHTML: entry.rawHTML,
                        extra: entry.extra
                    )
                    working[key] = entry
                }
            case .deleted:
                working.removeValue(forKey: key)
            }
        }
    }

    func entriesPage(after key: String?, limit: Int) throws -> [DictionaryEntry] {
        let keys = working.keys.filter { !unloadedKeys.contains($0) }.sorted()
        let start = key.map { cursor in keys.firstIndex { $0 > cursor } ?? keys.count } ?? 0
        return keys[start..<min(start + limit, keys.count)].compactMap { working[$0] }
    }

    func search(_ query: String, limit: Int) throws -> [SearchHit] {
        let keys = working.keys.filter { $0.hasPrefix(query) }.sorted().prefix(limit)
        return keys.map { SearchHit(key: $0, display: working[$0]?.display ?? $0) }
    }

    /// Mirrors `OdcDictionary.entry(for:)`: exact lookup by normalised key.
    func entry(for key: String) throws -> DictionaryEntry {
        let normalized = DictionaryEditorModel.normalizedKey(key)
        if lookupFailures.contains(normalized) {
            throw OdcError.entryFailed("repository unavailable")
        }
        guard let match = working.first(where: {
            DictionaryEditorModel.normalizedKey($0.key) == normalized
        }) else {
            // Same shape `OdcDictionary` produces for a missing key.
            throw OdcError.entryFailed("entry not found: \(key)")
        }
        return match.value
    }

    func upsert(_ entry: DictionaryEntry) throws -> EntryWriteResult {
        if failOnKeys.contains(entry.key) {
            throw OdcError.editFailed("upsert refused \(entry.key)")
        }
        let outcome: EntryWriteOutcome
        if working[entry.key] == nil {
            outcome = .added
        } else {
            outcome = .modified
        }
        working[entry.key] = entry
        return EntryWriteResult(key: entry.key, outcome: outcome)
    }

    func deleteEntry(_ key: String) throws -> EntryWriteResult {
        if failOnKeys.contains(key) {
            throw OdcError.editFailed("delete refused \(key)")
        }
        guard working.removeValue(forKey: key) != nil else {
            return EntryWriteResult(key: key, outcome: .notFound)
        }
        return EntryWriteResult(key: key, outcome: .deleted)
    }

    func workingStatus() throws -> WorkingStatus {
        let added = working.keys.filter { head[$0] == nil }.count
        let removed = head.keys.filter { working[$0] == nil }.count
        let modified = working.keys.filter { key in
            guard let before = head[key], let after = working[key] else { return false }
            return before != after
        }.count
        return WorkingStatus(
            added: added,
            modified: modified,
            removed: removed,
            total: working.count,
            entries: working.count,
            blocks: 0,
            head: "head"
        )
    }

    func workingChanges(after key: String?, limit: Int) throws -> [ChangeRow] {
        let keys = working.keys.sorted().filter { key == nil || $0 > key! }
        var rows: [ChangeRow] = []
        for candidate in keys {
            if let entry = working[candidate] {
                if head[candidate] == nil {
                    rows.append(ChangeRow(key: candidate, kind: .added))
                } else if head[candidate] != entry {
                    rows.append(ChangeRow(key: candidate, kind: .modified))
                }
            }
        }
        for candidate in head.keys.sorted() where working[candidate] == nil {
            rows.append(ChangeRow(key: candidate, kind: .deleted))
        }
        let sorted = rows.sorted { $0.key < $1.key }
        return Array(sorted.prefix(limit))
    }

    func entryDiff(_ key: String) throws -> EntryDiff {
        if let diff = diffs[key] {
            return diff
        }
        if head[key] == nil, working[key] != nil {
            return EntryDiff(key: key, kind: .added, fields: [])
        }
        if head[key] != nil, working[key] == nil {
            return EntryDiff(key: key, kind: .deleted, fields: [])
        }
        return EntryDiff(key: key, kind: .modified, fields: [])
    }

    func commit(message: String, author: String) throws -> CommitResult {
        commitMessage = message
        commitAuthor = author
        head = working
        return CommitResult(
            hash: "commit",
            entryCount: working.count,
            ts: 0,
            author: author,
            message: message,
            newBlocks: 0,
            reusedBlocks: 0,
            entries: working.count
        )
    }

    func discardWorking() throws -> DiscardResult {
        discardCount += 1
        working = head
        return DiscardResult(discarded: true)
    }
}

func makeEntry(_ key: String, display: String? = nil, sense: String = "sense") -> DictionaryEntry {
    DictionaryEntry(
        key: key,
        display: display ?? key.capitalized,
        phonetic: [],
        defs: [Definition(pos: "noun", sense: sense)],
        rawHTML: nil,
        extra: [:]
    )
}
