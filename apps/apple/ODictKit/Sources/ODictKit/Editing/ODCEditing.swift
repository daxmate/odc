import Foundation
import ODictCore

/// The subset of `OdcDictionary` the desktop editor uses.
///
/// Keeping the surface a protocol lets the editor view model run against a
/// fake in unit tests without opening a repository.
@MainActor
public protocol ODCEditing: AnyObject {
    func entriesPage(after key: String?, limit: Int) throws -> [DictionaryEntry]
    func search(_ query: String, limit: Int) throws -> [SearchHit]
    func entry(for key: String) throws -> DictionaryEntry
    func upsert(_ entry: DictionaryEntry) throws -> EntryWriteResult
    func deleteEntry(_ key: String) throws -> EntryWriteResult
    func workingStatus() throws -> WorkingStatus
    func workingChanges(after key: String?, limit: Int) throws -> [ChangeRow]
    func entryDiff(_ key: String) throws -> EntryDiff
    func commit(message: String, author: String) throws -> CommitResult
    func discardWorking() throws -> DiscardResult
}

extension OdcDictionary: ODCEditing {}
