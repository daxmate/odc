import Foundation

/// One row of a prefix search: the normalised key and the spelling to show.
public struct SearchHit: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let key: String
    public let display: String

    public var id: String { key }

    public init(key: String, display: String) {
        self.key = key
        self.display = display
    }
}

/// How the hits in a group matched the query.
public enum SearchMatchKind: Hashable, Sendable {
    /// Keys start with the query.
    case prefix
    /// Keys or display forms contain the query anywhere, including the start.
    case substring
}

/// What one dictionary contributed to the current search.
///
/// Groups follow the order the dictionaries were opened in, so the sidebar
/// does not reshuffle when a slower dictionary answers late.
public struct DictionarySearchGroup: Identifiable, Hashable, Sendable {
    /// Library identifier of the dictionary that produced `hits`.
    public let dictionaryID: UUID
    /// Display name, copied so the group stays readable after the item is gone.
    public let name: String
    public let hits: [SearchHit]
    /// Whether these hits are prefix matches or the fallback substring search.
    public let matchKind: SearchMatchKind

    public var id: UUID { dictionaryID }

    public init(
        dictionaryID: UUID,
        name: String,
        hits: [SearchHit],
        matchKind: SearchMatchKind = .prefix
    ) {
        self.dictionaryID = dictionaryID
        self.name = name
        self.hits = hits
        self.matchKind = matchKind
    }
}

/// A dictionary the reader has open.
///
/// `url` is the imported copy inside the library folder, the same directory
/// `documentURL` reports for the active dictionary.
public struct OpenDictionary: Identifiable, Hashable, Sendable {
    public let id: UUID
    public let name: String
    public let url: URL

    public init(id: UUID, name: String, url: URL) {
        self.id = id
        self.name = name
        self.url = url
    }
}

/// A single sense inside an entry. Mirrors `Def` in `odc-core`.
public struct Definition: Codable, Equatable, Hashable, Sendable {
    /// Part of speech tag, e.g. `noun`.
    public let pos: String
    /// The definition text.
    public let sense: String
    /// Example sentences.
    public let examples: [String]
    /// Usage labels.
    public let labels: [String]

    public init(pos: String, sense: String, examples: [String] = [], labels: [String] = []) {
        self.pos = pos
        self.sense = sense
        self.examples = examples
        self.labels = labels
    }

    private enum CodingKeys: String, CodingKey {
        case pos, sense, examples, labels
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        pos = try container.decodeIfPresent(String.self, forKey: .pos) ?? ""
        sense = try container.decodeIfPresent(String.self, forKey: .sense) ?? ""
        examples = try container.decodeIfPresent([String].self, forKey: .examples) ?? []
        labels = try container.decodeIfPresent([String].self, forKey: .labels) ?? []
    }
}

/// A dictionary entry as returned by `odc_repo_entry_json`.
public struct DictionaryEntry: Codable, Equatable, Sendable, Identifiable {
    /// Normalised lookup key.
    public let key: String
    /// Display form shown to readers.
    public let display: String
    /// Pronunciation strings (IPA or otherwise).
    public let phonetic: [String]
    /// Definitions.
    public let defs: [Definition]
    /// Source HTML fallback for entries without structured definitions.
    public let rawHTML: String?
    /// Payload fields this client does not model, preserved by the core.
    public let extra: [String: JSONValue]

    public var id: String { key }

    public init(
        key: String,
        display: String,
        phonetic: [String] = [],
        defs: [Definition] = [],
        rawHTML: String? = nil,
        extra: [String: JSONValue] = [:]
    ) {
        self.key = key
        self.display = display
        self.phonetic = phonetic
        self.defs = defs
        self.rawHTML = rawHTML
        self.extra = extra
    }

    private enum CodingKeys: String, CodingKey {
        case key, display, phonetic, defs
        case rawHTML = "raw_html"
        case extra
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        key = try container.decode(String.self, forKey: .key)
        display = try container.decodeIfPresent(String.self, forKey: .display) ?? key
        phonetic = try container.decodeIfPresent([String].self, forKey: .phonetic) ?? []
        defs = try container.decodeIfPresent([Definition].self, forKey: .defs) ?? []
        rawHTML = try container.decodeIfPresent(String.self, forKey: .rawHTML)
        extra = try container.decodeIfPresent([String: JSONValue].self, forKey: .extra) ?? [:]
    }
}

/// A JSON value of unknown shape, needed for the `extra` payload fields.
public enum JSONValue: Codable, Equatable, Hashable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported JSON value"
            )
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .number(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }

    /// A short single-line rendering used by the detail view.
    public var displayText: String {
        switch self {
        case .null:
            return "null"
        case .bool(let value):
            return value ? "true" : "false"
        case .number(let value):
            if value.rounded() == value, value.magnitude < 1e15 {
                return String(Int64(value))
            }
            return String(value)
        case .string(let value):
            return value
        case .array(let values):
            return "[" + values.map(\.displayText).joined(separator: ", ") + "]"
        case .object(let values):
            let body = values.keys.sorted().map { "\($0): \(values[$0]?.displayText ?? "")" }
            return "{" + body.joined(separator: ", ") + "}"
        }
    }
}
