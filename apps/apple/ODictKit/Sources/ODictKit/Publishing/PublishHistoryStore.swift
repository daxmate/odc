import Foundation

/// What the app remembers about the last successful publish of one library.
public struct PublishRecord: Codable, Equatable, Sendable {
    /// HEAD commit of the release that was published.
    public let head: String
    /// Version label the user typed.
    public let label: String
    /// When the publish finished.
    public let publishedAt: Date

    public init(head: String, label: String, publishedAt: Date = Date()) {
        self.head = head
        self.label = label
        self.publishedAt = publishedAt
    }

    /// First seven characters of the commit, for compact captions.
    public var shortHead: String {
        String(head.prefix(7))
    }
}

/// Per-library "last publish" bookkeeping.
///
/// The data is tiny and non-secret, so UserDefaults is enough; the key mixes
/// a namespace prefix with the library identifier so two dictionaries never
/// share a record.
@MainActor
public final class PublishHistoryStore {
    /// Prefix of every defaults key this type writes.
    public static let keyPrefix = "com.daxmate.odict.publish.last."

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// The last successful publish for `libraryKey`, if any.
    public func record(for libraryKey: String) -> PublishRecord? {
        guard let data = defaults.data(forKey: key(for: libraryKey)) else { return nil }
        return try? JSONDecoder().decode(PublishRecord.self, from: data)
    }

    /// Store (or replace) the record for `libraryKey`.
    public func update(_ record: PublishRecord, for libraryKey: String) {
        guard let data = try? JSONEncoder().encode(record) else { return }
        defaults.set(data, forKey: key(for: libraryKey))
    }

    /// Drop the record, for example after the library is deleted.
    public func clear(for libraryKey: String) {
        defaults.removeObject(forKey: key(for: libraryKey))
    }

    /// Defaults key for one library.
    public func key(for libraryKey: String) -> String {
        Self.keyPrefix + libraryKey
    }
}
