import Foundation

/// A named column mapping that can be reused for files with the same header.
public struct CSVColumnMappingTemplate: Codable, Equatable, Sendable, Identifiable {
    public var id: String { name }
    public let name: String
    /// Header the mapping was saved for; used for automatic matching.
    public let header: [String]
    public let mapping: CSVColumnMapping

    public init(name: String, header: [String], mapping: CSVColumnMapping) {
        self.name = name
        self.header = header
        self.mapping = mapping
    }
}

/// Stores column mapping templates per library.
///
/// Templates are tiny and contain no user content beyond header names, so they
/// live in UserDefaults next to the publish history; the defaults key carries
/// the library identifier so dictionaries on the same Mac stay isolated.
@MainActor
public final class CSVColumnTemplateStore {
    public static let keyPrefix = "com.daxmate.odict.csv.templates."

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public func templates(for libraryKey: String) -> [CSVColumnMappingTemplate] {
        guard let data = defaults.data(forKey: key(for: libraryKey)) else { return [] }
        return (try? JSONDecoder().decode([CSVColumnMappingTemplate].self, from: data)) ?? []
    }

    /// Save or replace a template with the same name.
    public func save(_ template: CSVColumnMappingTemplate, for libraryKey: String) {
        var current = templates(for: libraryKey)
        current.removeAll { $0.name == template.name }
        current.append(template)
        current.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        guard let data = try? JSONEncoder().encode(current) else { return }
        defaults.set(data, forKey: key(for: libraryKey))
    }

    public func delete(named name: String, for libraryKey: String) {
        var current = templates(for: libraryKey)
        current.removeAll { $0.name == name }
        guard let data = try? JSONEncoder().encode(current) else { return }
        defaults.set(data, forKey: key(for: libraryKey))
    }

    /// First template whose header matches, ignoring case and stray spaces.
    public func match(header: [String], for libraryKey: String) -> CSVColumnMappingTemplate? {
        let normalized = Self.normalized(header)
        guard !normalized.isEmpty else { return nil }
        return templates(for: libraryKey).first { Self.normalized($0.header) == normalized }
    }

    public func key(for libraryKey: String) -> String {
        Self.keyPrefix + libraryKey
    }

    static func normalized(_ header: [String]) -> [String] {
        header.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    }
}
