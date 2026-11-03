import Foundation
import ODCFFI

/// Errors surfaced by the C ABI wrapper.
public enum OdcError: LocalizedError, Equatable {
    /// `odc_repo_open` refused the path.
    case openFailed(String)
    /// `odc_repo_search_json` failed.
    case searchFailed(String)
    /// `odc_repo_entry_json` failed (including "entry not found").
    case entryFailed(String)
    /// An editing call (`put`, `delete`, `commit`, `diff`, `discard`) failed.
    case editFailed(String)
    /// `odc_repo_resource_len` / `odc_repo_resource_copy` failed.
    case resourceFailed(String)
    /// `limit` was outside the range the C ABI accepts.
    case invalidLimit(Int)
    /// `limit` was outside 1…500 for a paged editing call.
    case invalidPageLimit(Int)
    /// The dictionary was already closed.
    case closed
    /// The core returned JSON this client could not decode.
    case invalidResponse(String)

    public var errorDescription: String? {
        switch self {
        case .openFailed(let detail):
            return "无法打开词典：\(detail)"
        case .searchFailed(let detail):
            return "搜索失败：\(detail)"
        case .resourceFailed(let detail):
            return "读取资源失败：\(detail)"
        case .entryFailed(let detail):
            return "读取词条失败：\(detail)"
        case .editFailed(let detail):
            return "编辑词条失败：\(detail)"
        case .invalidLimit(let limit):
            return "搜索数量 \(limit) 超出 1…200 的范围。"
        case .invalidPageLimit(let limit):
            return "分页数量 \(limit) 超出 1…500 的范围。"
        case .closed:
            return "词典已关闭。"
        case .invalidResponse(let detail):
            return "词典返回了无法解析的数据：\(detail)"
        }
    }
}

/// Owns one OdcRepo pointer and closes it exactly once, when released.
///
/// The C handle is not thread safe, so the box is only touched through
/// OdcDictionary, which is confined to the main actor.
private final class RepoHandle: @unchecked Sendable {
    let pointer: OpaquePointer

    init(pointer: OpaquePointer) {
        self.pointer = pointer
    }

    deinit {
        odc_repo_close(pointer)
    }
}

/// Outcome of a lookup that follows `@@@LINK=` redirects.
public enum EntryLinkResult: Equatable, Sendable {
    /// An entry was found; `requestedKey` is the key the reader asked for and
    /// `hops` the number of redirects followed to reach it.
    case entry(DictionaryEntry, requestedKey: String, hops: Int)
    /// A redirect pointed at a word the dictionary does not carry.
    case missingTarget(String, requestedKey: String)
    /// The redirect chain came back to a word it had already visited.
    case cycle([String])
    /// The redirect chain was longer than the core allows.
    case tooLong([String])
    /// The word itself is absent.
    case notFound(String)
    /// HEAD still carries the word but the working set removed it.
    case deleted(String)
}

/// Thin, main-actor-confined wrapper around the ODCFFI C ABI.
///
/// An `OdcRepo` handle is **not** thread safe: the C header states it may be
/// used by one thread at a time. Confining the wrapper to the main actor keeps
/// every call on a single thread.
@MainActor
public final class OdcDictionary {
    private var repo: RepoHandle?
    private let decoder = JSONDecoder()

    /// Open an existing repository directory (one that contains `odc.db`).
    public init(url: URL) throws {
        var error: UnsafeMutablePointer<CChar>?
        let opened = url.path.withCString { path in
            odc_repo_open(path, &error)
        }
        guard let opened else {
            throw OdcError.openFailed(Self.take(error) ?? "无法读取 \(url.lastPathComponent)")
        }
        repo = RepoHandle(pointer: opened)
    }

    public var isOpen: Bool { repo != nil }

    /// Prefix search over normalised keys. `limit` must be within 1…200.
    public func search(_ query: String, limit: Int = 50) throws -> [SearchHit] {
        guard (1...200).contains(limit) else {
            throw OdcError.invalidLimit(limit)
        }
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = query.withCString { query in
            odc_repo_search_json(handle, query, Int32(limit), &error)
        }
        guard let result else {
            throw OdcError.searchFailed(Self.take(error) ?? "搜索失败")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try decoder.decode([SearchHit].self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    /// Substring search over normalised keys and display spellings.
    ///
    /// Hits are ordered shortest key first; `limit` must be within 1…200.
    public func searchSubstring(_ query: String, limit: Int = 50) throws -> [SearchHit] {
        guard (1...200).contains(limit) else {
            throw OdcError.invalidLimit(limit)
        }
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = query.withCString { query in
            odc_repo_search_substring_json(handle, query, Int32(limit), &error)
        }
        guard let result else {
            throw OdcError.searchFailed(Self.take(error) ?? "搜索失败")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try decoder.decode([SearchHit].self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    /// Exact lookup by normalised key.
    public func entry(for key: String) throws -> DictionaryEntry {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_json(handle, key, &error)
        }
        guard let result else {
            throw OdcError.entryFailed(Self.take(error) ?? "未找到词条：\(key)")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try decoder.decode(DictionaryEntry.self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    /// Exact lookup that follows redirects.
    ///
    /// MDict writes a cross-reference as a body of `@@@LINK=<target>`; this
    /// returns the entry the chain ends at, the number of hops it took, or the
    /// reason the chain did not settle.
    public func resolve(_ key: String) throws -> EntryLinkResult {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_link_json(handle, key, &error)
        }
        guard let result else {
            throw OdcError.entryFailed(Self.take(error) ?? "未找到词条：\(key)")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        return try decodeLink(data)
    }

    /// Same lookup as ``resolve(_:)`` but working-set edits win.
    ///
    /// A saved but uncommitted edit is returned instead of the committed
    /// body, so the reader shows the change immediately. A word removed from
    /// the working set reports ``EntryLinkResult/deleted(_:)`` and callers
    /// must not fall back to the committed entry.
    public func resolvePreferWorking(_ key: String) throws -> EntryLinkResult {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_link_working_json(handle, key, &error)
        }
        guard let result else {
            throw OdcError.entryFailed(Self.take(error) ?? "未找到词条：\(key)")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        return try decodeLink(data)
    }

    /// Decode the JSON both link entry points return.
    private func decodeLink(_ data: Data) throws -> EntryLinkResult {
        let payload: LinkPayload
        do {
            payload = try decoder.decode(LinkPayload.self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
        switch payload.status {
        case "found":
            guard let entry = payload.entry else {
                throw OdcError.invalidResponse("found without an entry")
            }
            return .entry(
                entry,
                requestedKey: payload.requested,
                hops: payload.hops ?? 0
            )
        case "missing":
            return .missingTarget(
                payload.target ?? "",
                requestedKey: payload.requested
            )
        case "cycle":
            return .cycle(payload.chain ?? [])
        case "too_long":
            return .tooLong(payload.chain ?? [])
        case "not_found":
            return .notFound(payload.requested)
        case "deleted":
            return .deleted(payload.requested)
        default:
            throw OdcError.invalidResponse("unknown status: \(payload.status)")
        }
    }

    /// JSON shape of `odc_repo_entry_link_json`.
    private struct LinkPayload: Decodable {
        let status: String
        let requested: String
        let key: String?
        let hops: Int?
        let target: String?
        let chain: [String]?
        let entry: DictionaryEntry?
    }

    /// Release the underlying handle. Safe to call more than once.
    public func close() {
        repo = nil
    }


    /// Payload stored for `key`, or `nil` when the dictionary has no such
    /// resource.
    ///
    /// The key is normalised by the core (backslashes, leading slashes and
    /// case are all tolerated), so `pic/apple.png` and `\\pic\\Apple.png`
    /// reach the same payload.
    public func resource(for key: String) throws -> Data? {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        var length: UInt64 = 0
        let found = key.withCString { keyPointer in
            odc_repo_resource_len(handle, keyPointer, &length, &error)
        }
        switch found {
        case 0:
            return nil
        case 1:
            break
        default:
            throw OdcError.resourceFailed(Self.take(error) ?? "无法读取资源：\(key)")
        }
        let count = Int(length)
        var bytes = [UInt8](repeating: 0, count: count)
        var copied = 0
        let code = bytes.withUnsafeMutableBufferPointer { buffer in
            key.withCString { keyPointer in
                odc_repo_resource_copy(
                    handle,
                    keyPointer,
                    buffer.baseAddress,
                    buffer.count,
                    &copied,
                    &error
                )
            }
        }
        guard code == 1 else {
            throw OdcError.resourceFailed(Self.take(error) ?? "无法读取资源：\(key)")
        }
        return Data(bytes.prefix(copied))
    }
    // MARK: - Editing

    /// One page of working-set entries in ascending key order.
    ///
    /// `limit` must be within 1…500. Pass the last entry's `key` as `after`
    /// to read the next page.
    public func entriesPage(after key: String? = nil, limit: Int = 200) throws -> [DictionaryEntry] {
        guard (1...500).contains(limit) else {
            throw OdcError.invalidPageLimit(limit)
        }
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result: UnsafeMutablePointer<CChar>?
        if let key {
            result = key.withCString { key in
                odc_repo_entries_page_json(handle, key, Int32(limit), &error)
            }
        } else {
            result = odc_repo_entries_page_json(handle, nil, Int32(limit), &error)
        }
        return try decode(result, error: error, context: "读取词条分页失败")
    }

    /// Insert or replace one entry in the working set.
    public func upsert(_ entry: DictionaryEntry) throws -> EntryWriteResult {
        let handle = try requireHandle()
        let data = try JSONEncoder().encode(entry)
        guard let text = String(data: data, encoding: .utf8) else {
            throw OdcError.editFailed("词条无法编码为 UTF-8")
        }
        var error: UnsafeMutablePointer<CChar>?
        let result = text.withCString { text in
            odc_repo_entry_put_json(handle, text, &error)
        }
        return try decode(result, error: error, context: "保存词条失败")
    }

    /// Remove one entry from the working set.
    public func deleteEntry(_ key: String) throws -> EntryWriteResult {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_delete_json(handle, key, &error)
        }
        return try decode(result, error: error, context: "删除词条失败")
    }

    /// Working set relative to HEAD.
    public func workingStatus() throws -> WorkingStatus {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = odc_repo_working_status_json(handle, &error)
        return try decode(result, error: error, context: "读取编辑状态失败")
    }

    /// Changed keys against HEAD, in ascending key order.
    public func workingChanges(after key: String? = nil, limit: Int = 200) throws -> [ChangeRow] {
        guard (1...500).contains(limit) else {
            throw OdcError.invalidPageLimit(limit)
        }
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result: UnsafeMutablePointer<CChar>?
        if let key {
            result = key.withCString { key in
                odc_repo_working_changes_json(handle, key, Int32(limit), &error)
            }
        } else {
            result = odc_repo_working_changes_json(handle, nil, Int32(limit), &error)
        }
        return try decode(result, error: error, context: "读取变更列表失败")
    }

    /// Field-level diff of one key between HEAD and the working set.
    public func entryDiff(_ key: String) throws -> EntryDiff {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_diff_json(handle, key, &error)
        }
        return try decode(result, error: error, context: "读取词条差异失败")
    }

    /// Commit the working set with the current wall-clock timestamp.
    public func commit(message: String, author: String) throws -> CommitResult {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = message.withCString { message in
            author.withCString { author in
                odc_repo_commit_json(handle, message, author, &error)
            }
        }
        return try decode(result, error: error, context: "提交失败")
    }

    /// Discard every working-set change; history is untouched.
    public func discardWorking() throws -> DiscardResult {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = odc_repo_discard_json(handle, &error)
        return try decode(result, error: error, context: "丢弃改动失败")
    }

    /// Decode a JSON result, releasing the C string and raising the core's
    /// diagnostic on failure.
    private func decode<T: Decodable>(
        _ pointer: UnsafeMutablePointer<CChar>?,
        error: UnsafeMutablePointer<CChar>?,
        context: String
    ) throws -> T {
        guard let pointer else {
            throw OdcError.editFailed(Self.take(error) ?? context)
        }
        defer { odc_string_free(pointer) }
        let data = Data(bytes: pointer, count: strlen(pointer))
        do {
            return try decoder.decode(T.self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }
    private func requireHandle() throws -> OpaquePointer {
        guard let repo else { throw OdcError.closed }
        return repo.pointer
    }

    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { odc_string_free(pointer) }
        return String(cString: pointer)
    }
}
