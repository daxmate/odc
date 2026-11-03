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

/// Outcome of a single working-set write (`odc_repo_entry_put_json` /
/// `odc_repo_entry_delete_json`).
public enum EntryWriteOutcome: String, Codable, Equatable, Sendable {
    case added
    case modified
    case deleted
    case notFound = "not_found"
}

/// Result of writing or deleting one working-set entry.
public struct EntryWriteResult: Codable, Equatable, Sendable {
    public let key: String
    public let outcome: EntryWriteOutcome

    public init(key: String, outcome: EntryWriteOutcome) {
        self.key = key
        self.outcome = outcome
    }
}

/// Working set relative to HEAD (`odc_repo_working_status_json`).
public struct WorkingStatus: Codable, Equatable, Sendable {
    /// Keys present only in the working set.
    public let added: Int
    /// Keys whose content hash differs from HEAD.
    public let modified: Int
    /// Keys present in HEAD but deleted from the working set.
    public let removed: Int
    /// Working-set size.
    public let total: Int
    /// Entries materialised in the committed index.
    public let entries: Int
    /// Blocks stored by the repository.
    public let blocks: Int
    /// Current HEAD hash, absent before the first commit.
    public let head: String?

    public var isClean: Bool { added == 0 && modified == 0 && removed == 0 }

    public init(
        added: Int, modified: Int, removed: Int, total: Int,
        entries: Int, blocks: Int, head: String?
    ) {
        self.added = added
        self.modified = modified
        self.removed = removed
        self.total = total
        self.entries = entries
        self.blocks = blocks
        self.head = head
    }
}

/// What happened to one key between HEAD and the working set.
public enum ChangeKind: String, Codable, Equatable, Sendable {
    case added
    case modified
    case deleted
}

/// One row of `odc_repo_working_changes_json`.
public struct ChangeRow: Codable, Equatable, Sendable, Identifiable {
    public let key: String
    public let kind: ChangeKind

    public var id: String { key }

    public init(key: String, kind: ChangeKind) {
        self.key = key
        self.kind = kind
    }
}

/// How an entry differs overall (`odc_repo_entry_diff_json`).
public enum EntryDiffKind: String, Codable, Equatable, Sendable {
    case added
    case modified
    case deleted
    case absent
}

/// One field of an `EntryDiff`.
///
/// `before` and `after` are `nil` when the field does not exist on that
/// side, so an added definition has no `before` for any of its attributes.
public struct FieldDiff: Codable, Equatable, Sendable {
    /// Field name, e.g. `display`, `defs[0].sense` or `extra.source`.
    public let field: String
    /// Value in HEAD, if the field exists there.
    public let before: JSONValue?
    /// Value in the working set, if the field exists there.
    public let after: JSONValue?
    /// Whether the two sides differ.
    public let changed: Bool

    public init(field: String, before: JSONValue?, after: JSONValue?, changed: Bool) {
        self.field = field
        self.before = before
        self.after = after
        self.changed = changed
    }
}

/// Field-level diff of one entry between HEAD and the working set.
public struct EntryDiff: Codable, Equatable, Sendable {
    public let key: String
    public let kind: EntryDiffKind
    public let fields: [FieldDiff]

    public var changedFields: [FieldDiff] { fields.filter(\.changed) }

    public init(key: String, kind: EntryDiffKind, fields: [FieldDiff]) {
        self.key = key
        self.kind = kind
        self.fields = fields
    }
}

/// Result of `odc_repo_commit_json`.
public struct CommitResult: Codable, Equatable, Sendable {
    /// New commit hash, hex encoded.
    public let hash: String
    /// Entries in the committed snapshot.
    public let entryCount: Int
    /// Commit timestamp, seconds since the Unix epoch.
    public let ts: Int64
    public let author: String
    public let message: String
    /// Blocks written by this commit.
    public let newBlocks: Int
    /// Blocks reused from earlier commits.
    public let reusedBlocks: Int
    /// Entries in the snapshot (same value as `entryCount`).
    public let entries: Int

    private enum CodingKeys: String, CodingKey {
        case hash, ts, author, message, entries
        case entryCount = "entry_count"
        case newBlocks = "new_blocks"
        case reusedBlocks = "reused_blocks"
    }

    public init(
        hash: String, entryCount: Int, ts: Int64, author: String, message: String,
        newBlocks: Int, reusedBlocks: Int, entries: Int
    ) {
        self.hash = hash
        self.entryCount = entryCount
        self.ts = ts
        self.author = author
        self.message = message
        self.newBlocks = newBlocks
        self.reusedBlocks = reusedBlocks
        self.entries = entries
    }
}

/// Result of `odc_repo_discard_json`.
public struct DiscardResult: Codable, Equatable, Sendable {
    public let discarded: Bool

    public init(discarded: Bool) {
        self.discarded = discarded
    }
}
